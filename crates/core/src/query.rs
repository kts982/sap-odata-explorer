use crate::metadata::ODataVersion;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Serialize;

/// Percent-encoding set matching JavaScript's `encodeURIComponent`: encode every
/// byte except `A-Z a-z 0-9 - _ . ~ ! * ' ( )`. Values containing `&`, `#`, `+`,
/// `=`, `?`, or space would otherwise produce wrong requests or split the query
/// string — e.g. a filter value `'Berlin & Munich'` would be truncated at the `&`.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~')
    .remove(b'!')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

fn enc(s: &str) -> String {
    utf8_percent_encode(s, COMPONENT).to_string()
}

/// OData query builder that constructs URL query parameters.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ODataQuery {
    entity_set: String,
    key: Option<String>,
    select: Vec<String>,
    filter: Option<String>,
    expand: Vec<String>,
    orderby: Vec<String>,
    top: Option<u32>,
    skip: Option<u32>,
    count: bool,
    search: Option<String>,
    format: Option<String>,
    version: Option<ODataVersion>,
    custom: Vec<(String, String)>,
}

impl ODataQuery {
    /// Create a new query targeting an entity set.
    pub fn new(entity_set: &str) -> Self {
        Self {
            entity_set: entity_set.to_string(),
            ..Default::default()
        }
    }

    /// Set the entity key for a single-entity read (e.g., `CustomerSet('CUST01')`).
    pub fn key(mut self, key: &str) -> Self {
        self.key = Some(key.to_string());
        self
    }

    /// Add fields to $select.
    pub fn select(mut self, fields: &[&str]) -> Self {
        self.select.extend(fields.iter().map(|s| s.to_string()));
        self
    }

    /// Set the $filter expression.
    pub fn filter(mut self, expr: &str) -> Self {
        self.filter = Some(expr.to_string());
        self
    }

    /// Add navigation properties to $expand.
    pub fn expand(mut self, navs: &[&str]) -> Self {
        self.expand.extend(navs.iter().map(|s| s.to_string()));
        self
    }

    /// Add ordering clauses to $orderby (e.g., "Amount desc").
    pub fn orderby(mut self, clauses: &[&str]) -> Self {
        self.orderby.extend(clauses.iter().map(|s| s.to_string()));
        self
    }

    /// Set the $top limit.
    pub fn top(mut self, n: u32) -> Self {
        self.top = Some(n);
        self
    }

    /// Set $skip offset.
    pub fn skip(mut self, n: u32) -> Self {
        self.skip = Some(n);
        self
    }

    /// Request inline count ($inlinecount=allpages for V2, $count=true for V4).
    pub fn count(mut self) -> Self {
        self.count = true;
        self
    }

    /// Set the free-text search term. V4 emits `$search="term"` (with
    /// double quotes per spec), V2 falls back to SAP's legacy
    /// `search=term` custom query param.
    pub fn search(mut self, term: &str) -> Self {
        self.search = Some(term.to_string());
        self
    }

    /// Set the OData version (affects query syntax).
    pub fn version(mut self, v: ODataVersion) -> Self {
        self.version = Some(v);
        self
    }

    /// Set the $format (json or xml).
    pub fn format(mut self, fmt: &str) -> Self {
        self.format = Some(fmt.to_string());
        self
    }

    /// Add a custom query parameter.
    pub fn custom(mut self, key: &str, value: &str) -> Self {
        self.custom.push((key.to_string(), value.to_string()));
        self
    }

    /// Build the relative URL path + query string.
    pub fn build(&self) -> String {
        let mut path = self.entity_set.clone();

        if let Some(ref key) = self.key {
            path.push('(');
            path.push_str(key);
            path.push(')');
        }

        let mut params: Vec<String> = Vec::new();

        if !self.select.is_empty() {
            params.push(format!("$select={}", enc(&self.select.join(","))));
        }
        if let Some(ref filter) = self.filter {
            params.push(format!("$filter={}", enc(filter)));
        }
        if !self.expand.is_empty() {
            params.push(format!("$expand={}", enc(&self.expand.join(","))));
        }
        if !self.orderby.is_empty() {
            params.push(format!("$orderby={}", enc(&self.orderby.join(","))));
        }
        if let Some(top) = self.top {
            params.push(format!("$top={top}"));
        }
        if let Some(skip) = self.skip {
            params.push(format!("$skip={skip}"));
        }
        if self.count {
            match self.version {
                Some(ODataVersion::V4) => params.push("$count=true".to_string()),
                _ => params.push("$inlinecount=allpages".to_string()),
            }
        }
        if let Some(ref term) = self.search {
            // V4 requires the search phrase in double quotes. V2 SAP
            // services accept a bare `search=term` custom param.
            match self.version {
                Some(ODataVersion::V4) => {
                    params.push(format!("$search={}", enc(&format!("\"{term}\""))));
                }
                _ => {
                    params.push(format!("search={}", enc(term)));
                }
            }
        }
        if let Some(ref fmt) = self.format {
            params.push(format!("$format={}", enc(fmt)));
        }
        for (k, v) in &self.custom {
            params.push(format!("{}={}", enc(k), enc(v)));
        }

        if params.is_empty() {
            path
        } else {
            format!("{path}?{}", params.join("&"))
        }
    }

    /// Get the entity set name.
    pub fn entity_set(&self) -> &str {
        &self.entity_set
    }
}

impl std::fmt::Display for ODataQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.build())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_query() {
        let q = ODataQuery::new("CustomerSet");
        assert_eq!(q.build(), "CustomerSet");
    }

    #[test]
    fn test_key_access() {
        let q = ODataQuery::new("CustomerSet").key("'CUST01'");
        assert_eq!(q.build(), "CustomerSet('CUST01')");
    }

    #[test]
    fn test_full_query() {
        let q = ODataQuery::new("CustomerSet")
            .select(&["CustomerID", "CustomerName"])
            .filter("City eq 'Berlin'")
            .expand(&["ToOrders"])
            .orderby(&["CustomerName asc"])
            .top(10)
            .skip(0)
            .count()
            .format("json");

        let result = q.build();
        assert!(result.starts_with("CustomerSet?"));
        assert!(result.contains("$select=CustomerID%2CCustomerName"));
        assert!(result.contains("$filter=City%20eq%20'Berlin'"));
        assert!(result.contains("$expand=ToOrders"));
        assert!(result.contains("$orderby=CustomerName%20asc"));
        assert!(result.contains("$top=10"));
        assert!(result.contains("$skip=0"));
        assert!(result.contains("$inlinecount=allpages"));
        assert!(result.contains("$format=json"));
    }

    #[test]
    fn test_custom_params() {
        let q = ODataQuery::new("OrderSet").custom("search", "test");
        assert_eq!(q.build(), "OrderSet?search=test");
    }

    #[test]
    fn test_search_v4_quotes_term() {
        let q = ODataQuery::new("OrderSet")
            .version(ODataVersion::V4)
            .search("HB");
        // Double quotes are part of V4 syntax but not URI-safe — encode them.
        assert!(q.build().contains("$search=%22HB%22"));
    }

    #[test]
    fn test_search_v2_uses_bare_term() {
        let q = ODataQuery::new("OrderSet")
            .version(ODataVersion::V2)
            .search("HB");
        assert!(q.build().contains("search=HB"));
        assert!(!q.build().contains("$search"));
    }

    #[test]
    fn test_filter_encodes_ampersand() {
        // Without encoding, the `&` in the value would split the query string
        // and the server would see `$filter=City eq 'Berlin ` plus a stray
        // `Munich'` param — wrong results or a parse error.
        let q = ODataQuery::new("CitySet").filter("Name eq 'Berlin & Munich'");
        let result = q.build();
        assert!(result.contains("$filter=Name%20eq%20'Berlin%20%26%20Munich'"));
        assert_eq!(result.matches('&').count(), 0);
    }

    #[test]
    fn test_filter_encodes_space_and_plus() {
        let q = ODataQuery::new("EventSet").filter("Note eq 'a + b = c'");
        let result = q.build();
        // Space → %20, + → %2B, = → %3D. Single quotes stay (unreserved in our set).
        assert!(result.contains("$filter=Note%20eq%20'a%20%2B%20b%20%3D%20c'"));
    }

    #[test]
    fn test_filter_preserves_apostrophe_literal() {
        // OData escapes apostrophes by doubling them: O'Brien → 'O''Brien'.
        // Apostrophe is in our preserve set, so the doubled-quote form survives.
        let q = ODataQuery::new("PersonSet").filter("Name eq 'O''Brien'");
        let result = q.build();
        assert!(result.contains("$filter=Name%20eq%20'O''Brien'"));
    }

    #[test]
    fn test_search_v4_encodes_space() {
        let q = ODataQuery::new("ProductSet")
            .version(ODataVersion::V4)
            .search("red car");
        assert!(q.build().contains("$search=%22red%20car%22"));
    }

    #[test]
    fn test_custom_param_encodes_value() {
        let q = ODataQuery::new("X").custom("note", "a=b&c");
        assert_eq!(q.build(), "X?note=a%3Db%26c");
    }
}
