use std::collections::HashSet;

use serde::Serialize;
use tracing::debug;

use crate::client::SapClient;
use crate::error::ODataError;

/// A registered OData service from the SAP Gateway catalog.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogEntry {
    pub title: String,
    pub technical_name: String,
    pub version: String,
    pub description: String,
    pub service_url: String,
    pub is_v4: bool,
}

const V2_CATALOG_PATH: &str = "/sap/opu/odata/IWFND/CATALOGSERVICE;v=2";
const V4_CATALOG_PATH: &str = "/sap/opu/odata4/iwfnd/config/default/iwfnd/catalog/0002";

/// Result of a catalog fetch including any warnings.
pub struct CatalogResult {
    pub entries: Vec<CatalogEntry>,
    pub warnings: Vec<String>,
}

/// Fetch the list of all available OData services (V2 + V4) from the SAP Gateway catalogs.
/// Deduplicates by technical name (keeps the entry with a URL if one has it).
/// Returns warnings for catalog-level failures instead of silently swallowing them.
pub async fn fetch_service_catalog(client: &SapClient) -> Result<CatalogResult, ODataError> {
    let mut entries = Vec::new();
    let mut warnings = Vec::new();

    // Fetch V2 catalog
    match fetch_v2_catalog(client).await {
        Ok(v2) => entries.extend(v2),
        Err(e) => {
            let msg = format!("V2 catalog: {e}");
            debug!("{msg}");
            warnings.push(msg);
        }
    }

    // Fetch V4 catalog
    match fetch_v4_catalog(client).await {
        Ok(v4) => entries.extend(v4),
        Err(e) => {
            let msg = format!("V4 catalog: {e}");
            debug!("{msg}");
            warnings.push(msg);
        }
    }

    // Deduplicate: keep the entry with a non-empty URL when duplicates exist
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    // First pass: entries with URLs
    for entry in &entries {
        if !entry.service_url.is_empty() {
            let key = (entry.technical_name.to_lowercase(), entry.is_v4);
            if seen.insert(key) {
                deduped.push(entry.clone());
            }
        }
    }
    // Second pass: entries without URLs (only if not already seen)
    for entry in &entries {
        if entry.service_url.is_empty() {
            let key = (entry.technical_name.to_lowercase(), entry.is_v4);
            if seen.insert(key) {
                deduped.push(entry.clone());
            }
        }
    }

    Ok(CatalogResult {
        entries: deduped,
        warnings,
    })
}

async fn fetch_v2_catalog(client: &SapClient) -> Result<Vec<CatalogEntry>, ODataError> {
    client.ensure_session(V2_CATALOG_PATH).await?;

    let url_path = format!("{V2_CATALOG_PATH}/ServiceCollection?$format=json");
    let json_text = client.get_raw(V2_CATALOG_PATH, &url_path).await?;
    debug!(
        "V2 catalog raw response ({} bytes): {}",
        json_text.len(),
        &json_text[..json_text.len().min(500)]
    );
    let data: serde_json::Value = serde_json::from_str(&json_text)
        .map_err(|e| ODataError::MetadataParse(format!("V2 catalog parse error: {e}")))?;

    debug!("V2 catalog response received");

    // OData V2 envelope: { "d": { "results": [...] } }
    let results = data
        .get("d")
        .and_then(|d| d.get("results"))
        .and_then(|r| r.as_array());

    let Some(results) = results else {
        return Ok(vec![]);
    };

    Ok(results
        .iter()
        .map(|entry| CatalogEntry {
            title: str_field(entry, "Title"),
            technical_name: str_field(entry, "TechnicalServiceName"),
            version: str_field(entry, "TechnicalServiceVersion"),
            description: str_field(entry, "Description"),
            service_url: str_field(entry, "ServiceUrl"),
            is_v4: false,
        })
        .collect())
}

async fn fetch_v4_catalog(client: &SapClient) -> Result<Vec<CatalogEntry>, ODataError> {
    client.ensure_session(V4_CATALOG_PATH).await?;

    // Expand to get service URLs in one request. Pick first ServiceUrl per group.
    let url_path = format!(
        "{V4_CATALOG_PATH}/ServiceGroups?$expand=DefaultSystem($expand=Services)&$format=json"
    );
    let json_text = client.get_raw(V4_CATALOG_PATH, &url_path).await?;
    let data: serde_json::Value = serde_json::from_str(&json_text)
        .map_err(|e| ODataError::MetadataParse(format!("V4 catalog parse error: {e}")))?;

    debug!("V4 catalog response received");

    let groups = data.get("value").and_then(|v| v.as_array());

    let Some(groups) = groups else {
        return Ok(vec![]);
    };

    Ok(groups
        .iter()
        .map(|group| {
            let group_id = str_field(group, "GroupId");
            let description = str_field(group, "Description");

            // Extract the first real ServiceUrl from DefaultSystem/Services
            // (skip iwbep/common helper services)
            let service_url = group
                .get("DefaultSystem")
                .and_then(|ds| ds.get("Services"))
                .and_then(|s| s.as_array())
                .and_then(|services| {
                    services.iter().find_map(|svc| {
                        let url = str_field(svc, "ServiceUrl");
                        if is_real_v4_service_url(&url) {
                            Some(extract_path_from_url(&url))
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or_default();

            CatalogEntry {
                title: description.clone(),
                technical_name: group_id,
                version: String::new(),
                description,
                service_url,
                is_v4: true,
            }
        })
        .collect())
}

/// Resolve the actual service URL for a V4 service group by querying its services.
/// Queries: ServiceGroups('<group_id>')/DefaultSystem/Services
pub async fn resolve_v4_service_url(
    client: &SapClient,
    group_id: &str,
) -> Result<String, ODataError> {
    client.ensure_session(V4_CATALOG_PATH).await?;

    // URL-encode the group ID for the key (single quotes around it)
    let url_path = format!(
        "{V4_CATALOG_PATH}/ServiceGroups('{group_id}')/DefaultSystem/Services?$format=json"
    );
    let json_text = client.get_raw(V4_CATALOG_PATH, &url_path).await?;
    let data: serde_json::Value = serde_json::from_str(&json_text)
        .map_err(|e| ODataError::MetadataParse(format!("V4 service resolve error: {e}")))?;

    debug!("V4 service resolution for '{group_id}' received");

    // Pick the first real service URL (skip iwbep/common helpers)
    let services = data.get("value").and_then(|v| v.as_array());
    if let Some(services) = services {
        for svc in services {
            let url = str_field(svc, "ServiceUrl");
            if is_real_v4_service_url(&url) {
                return Ok(extract_path_from_url(&url));
            }
        }
    }

    Err(ODataError::ServiceNotFound(format!(
        "no service URL found for V4 group '{group_id}'"
    )))
}

/// Resolve a service name to its full URL path by searching both V2 and V4 catalogs.
/// Returns the service URL if found.
pub async fn resolve_service_by_name(client: &SapClient, name: &str) -> Result<String, ODataError> {
    let name_lower = name.to_lowercase();
    let mut errors = Vec::new();

    // Try V2 catalog first — it already has ServiceUrl
    match fetch_v2_catalog(client).await {
        Ok(v2_entries) => {
            for entry in &v2_entries {
                if entry.technical_name.to_lowercase() == name_lower {
                    if !entry.service_url.is_empty() {
                        let path = extract_path_from_url(&entry.service_url);
                        debug!("Resolved '{name}' via V2 catalog: {path}");
                        return Ok(path);
                    }
                }
            }
        }
        Err(e) => {
            debug!("V2 catalog fetch failed during resolve: {e}");
            errors.push(format!("V2: {e}"));
        }
    }

    // Try V4 catalog — need to resolve the URL via expand
    match fetch_v4_catalog(client).await {
        Ok(v4_entries) => {
            for entry in &v4_entries {
                if entry.technical_name.to_lowercase() == name_lower {
                    debug!("Found '{name}' in V4 catalog, resolving URL...");
                    match resolve_v4_service_url(client, &entry.technical_name).await {
                        Ok(url) => {
                            let path = extract_path_from_url(&url);
                            debug!("Resolved '{name}' via V4 catalog: {path}");
                            return Ok(path);
                        }
                        Err(e) => {
                            debug!("V4 URL resolution failed for '{name}': {e}");
                        }
                    }
                }
            }
        }
        Err(e) => {
            debug!("V4 catalog fetch failed during resolve: {e}");
            errors.push(format!("V4: {e}"));
        }
    }

    let detail = if errors.is_empty() {
        String::new()
    } else {
        format!(" (catalog errors: {})", errors.join("; "))
    };
    Err(ODataError::ServiceNotFound(format!(
        "service '{name}' not found in V2 or V4 catalogs{detail}"
    )))
}

/// Extract the path component from a URL, stripping scheme, host, and query params.
fn extract_path_from_url(url: &str) -> String {
    let path = if url.starts_with("http://") || url.starts_with("https://") {
        if let Ok(parsed) = url::Url::parse(url) {
            parsed.path().to_string()
        } else {
            url.to_string()
        }
    } else if let Some(idx) = url.find('?') {
        // Strip query params from relative URLs
        url[..idx].to_string()
    } else {
        url.to_string()
    };
    // Remove trailing slash
    path.trim_end_matches('/').to_string()
}

/// Check if a V4 service URL is the actual service (not a common/metadata helper).
fn is_real_v4_service_url(url: &str) -> bool {
    !url.is_empty() && !url.contains("/iwbep/common/") && !url.contains("/iwbep/tea_")
}

fn str_field(value: &serde_json::Value, field: &str) -> String {
    value
        .get(field)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

impl CatalogEntry {
    /// Check if this entry matches a search term (case-insensitive, matches name, title, or description).
    pub fn matches(&self, search: &str) -> bool {
        let search = search.to_lowercase();
        self.technical_name.to_lowercase().contains(&search)
            || self.title.to_lowercase().contains(&search)
            || self.description.to_lowercase().contains(&search)
    }

    /// Version label for display.
    pub fn version_label(&self) -> &str {
        if self.is_v4 { "V4" } else { "V2" }
    }
}
