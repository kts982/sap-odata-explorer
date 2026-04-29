//! Public model types for parsed `$metadata`.
//!
//! Pure data definitions + the `impl ServiceMetadata` accessors. No
//! parsing / XML / network code lives here — those concerns are in the
//! `metadata` module's `mod.rs` (and, in a follow-up split, `annotations`).
//!
//! `extract_type_name` and `parse_v4_nav_type` are kept here as
//! `pub(super)` helpers because they belong conceptually with type
//! manipulation (operating on qualified type strings) and the impl
//! methods need them. The `pub(super)` visibility keeps them out of the
//! crate's public surface but reachable from siblings inside the
//! `metadata` module.

use serde::Serialize;
use std::collections::BTreeMap;

/// Detected OData protocol version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ODataVersion {
    V2,
    V4,
}

/// Parsed OData service metadata.
#[derive(Debug, Clone, Serialize)]
pub struct ServiceMetadata {
    pub version: ODataVersion,
    pub schema_namespace: String,
    pub entity_types: Vec<EntityType>,
    pub associations: Vec<Association>,
    pub entity_sets: Vec<EntitySet>,
    pub function_imports: Vec<FunctionImport>,
    /// Raw annotations captured from `$metadata`. Flat list — each entry
    /// points at the thing it was attached to via `target`. V2 SAP inline
    /// attributes (`sap:*`) and V4 `<Annotations>` blocks both land here.
    /// Complex record/collection values are not expanded yet; `value` is
    /// populated only for the common simple-valued forms
    /// (`String=`, `Bool=`, `Int=`, `EnumMember=`).
    pub annotations: Vec<RawAnnotation>,
}

/// A single annotation captured during `$metadata` parsing. The first slice
/// of the annotation feature only needs counts + grouping; richer typed
/// accessors will layer on top of this without re-parsing.
#[derive(Debug, Clone, Serialize)]
pub struct RawAnnotation {
    /// Fully-qualified term name: `Common.Label`, `UI.LineItem`, or `sap:label`
    /// for V2 SAP inline attributes.
    pub term: String,
    /// Display-friendly vocabulary name derived from the term: `Common`, `UI`,
    /// `Capabilities`, `Measures`, `SAP`, ...
    pub namespace: String,
    /// What the annotation is attached to. For V4: the `Target` on the
    /// enclosing `<Annotations>` block with the schema alias stripped
    /// (e.g. `EntityType/PropertyName`). For V2: a synthetic path built
    /// from the containing element (`EntityType/PropertyName`,
    /// `EntityContainer/EntitySetName`, etc.).
    pub target: String,
    /// Literal value when the annotation uses a simple inline form. `None`
    /// for complex `Record`/`Collection` payloads, which are not expanded yet.
    pub value: Option<String>,
    /// Optional `Qualifier` — the same term can be annotated multiple times
    /// on the same target with different qualifiers (e.g. `UI.LineItem
    /// Qualifier="Simplified"`).
    pub qualifier: Option<String>,
}

/// Summary of annotation counts grouped by vocabulary namespace. Used by the
/// desktop app's footer badge and its hover summary.
#[derive(Debug, Clone, Serialize, Default)]
pub struct AnnotationSummary {
    pub total: usize,
    /// Namespace → count. BTreeMap keeps display order deterministic.
    pub by_namespace: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EntityType {
    pub name: String,
    pub keys: Vec<String>,
    pub properties: Vec<Property>,
    pub nav_properties: Vec<NavigationProperty>,
    /// Parsed `UI.HeaderInfo` annotation if the service exposes one (V4).
    /// Gives the entity's human-readable name and title field.
    pub header_info: Option<HeaderInfo>,
    /// Property names flagged as default filter fields by
    /// `UI.SelectionFields` — the columns a Fiori list report would
    /// expose as selection filters out of the box. Empty if the service
    /// doesn't specify any. Paths are stored verbatim (no alias resolution).
    pub selection_fields: Vec<String>,
    /// Columns a Fiori list report would show by default, from
    /// `UI.LineItem`. Only `UI.DataField` records with a `Value Path="..."`
    /// are surfaced here; `DataFieldFor*` variants (Action, Annotation,
    /// IntentBasedNavigation, ...) are ignored because they don't map to
    /// `$select`-able columns. Order matches the source collection.
    pub line_item: Vec<LineItemField>,
    /// Property paths listed under `UI.PresentationVariant.RequestAtLeast`
    /// — fields Fiori silently appends to `$select` regardless of which
    /// columns the user picks, typically technical supports like time
    /// zones and key descriptions. Used to augment the "Fiori cols"
    /// quick-action. Empty when the service doesn't declare any.
    pub request_at_least: Vec<String>,
    /// `UI.PresentationVariant.SortOrder` entries — the default ordering
    /// Fiori would apply to the list. Each entry points at a property
    /// with a direction flag. Order matches the source collection.
    /// Consumed by the "Fiori cols" action to also fill `$orderby`.
    pub sort_order: Vec<SortOrder>,
    /// Declared `UI.SelectionVariant` records on this entity — Fiori's
    /// "filter variants" (e.g. "Pending", "Completed"). Includes the
    /// default (no-qualifier) variant and any qualified ones. Used by
    /// the desktop's "Fiori filter" button to drop pre-declared
    /// filter defaults into `$filter`. Empty when the service declares
    /// no variants.
    pub selection_variants: Vec<SelectionVariant>,
    /// `Capabilities.SearchRestrictions.Searchable` — whether the set
    /// accepts `$search`. `None` means the service didn't declare it
    /// (callers assume `true` per OData default).
    pub searchable: Option<bool>,
    /// `Capabilities.CountRestrictions.Countable` — whether `$count`
    /// and `$inlinecount` are honored. `None` = unspecified.
    pub countable: Option<bool>,
    /// `Capabilities.TopSupported` — `$top` honored. `None` = unspecified.
    pub top_supported: Option<bool>,
    /// `Capabilities.SkipSupported` — `$skip` honored. `None` = unspecified.
    pub skip_supported: Option<bool>,
    /// Navigation property names listed under
    /// `Capabilities.ExpandRestrictions.NonExpandableProperties` — the
    /// server rejects `$expand` referencing any of these. Empty when
    /// the service declares no restrictions or marks the whole set
    /// non-expandable (see `expandable` for the global flag).
    pub non_expandable_properties: Vec<String>,
    /// `Capabilities.ExpandRestrictions.Expandable` — `$expand` is
    /// supported at all on this set. `None` = unspecified (assume true).
    pub expandable: Option<bool>,
    /// Property paths that form the *business* key, from
    /// `Common.SemanticKey`. Often smaller (and more human-meaningful)
    /// than the technical primary key — e.g. `Product` instead of a
    /// generated UUID. Empty when the service doesn't declare one.
    pub semantic_keys: Vec<String>,
}

/// One `UI.SelectionVariant` record — the declarative shape of a
/// Fiori filter variant. Contains single-valued `Parameters` (always
/// equality) and multi-valued `SelectOptions` (SELECT-OPTIONS-style
/// ranges with sign and option code).
#[derive(Debug, Clone, Serialize)]
pub struct SelectionVariant {
    /// Optional `Qualifier` on the annotation — distinguishes variants
    /// on the same entity (e.g. `Qualifier="Pending"`). `None` for the
    /// default variant.
    pub qualifier: Option<String>,
    /// `Text` property — human-readable variant name (e.g.
    /// `"Pending Orders"`). `None` if not declared.
    pub text: Option<String>,
    /// Fixed `property = value` assignments fed as filter constraints.
    pub parameters: Vec<SelectionParameter>,
    /// Per-property lists of SELECT-OPTIONS-style ranges.
    pub select_options: Vec<SelectOption>,
}

/// Single-valued parameter in a `UI.SelectionVariant.Parameters`.
/// Shape: `<Record Type="UI.ParameterType">` with `PropertyName` +
/// `PropertyValue`. Always interpreted as `property eq value`.
#[derive(Debug, Clone, Serialize)]
pub struct SelectionParameter {
    pub property_name: String,
    pub property_value: String,
}

/// One property's worth of SELECT-OPTIONS ranges inside
/// `UI.SelectionVariant.SelectOptions`. Multiple ranges on the same
/// property are OR-joined when building `$filter`.
#[derive(Debug, Clone, Serialize)]
pub struct SelectOption {
    pub property_name: String,
    pub ranges: Vec<SelectionRange>,
}

/// One `<Record Type="UI.SelectionRangeType">` inside a `SelectOption`.
/// Mirrors ABAP SELECT-OPTIONS semantics: Include/Exclude × operator.
#[derive(Debug, Clone, Serialize)]
pub struct SelectionRange {
    /// `I` (include, normal comparison) or `E` (exclude, negated).
    pub sign: SelectionSign,
    /// Comparison operator — `EQ`/`NE`/`GT`/`GE`/`LT`/`LE`/`BT`/`NB`/`CP`/`NP`.
    pub option: SelectionOption,
    /// Primary value. For `BT`/`NB` this is the lower bound.
    pub low: String,
    /// Upper bound — only present for `BT`/`NB`. `None` otherwise.
    pub high: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SelectionSign {
    I,
    E,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SelectionOption {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    Bt,
    Nb,
    Cp,
    Np,
}

/// One entry inside `UI.PresentationVariant.SortOrder`. Maps directly
/// to an `$orderby` clause: `property [asc|desc]`.
#[derive(Debug, Clone, Serialize)]
pub struct SortOrder {
    pub property: String,
    pub descending: bool,
}

/// One `UI.DataField` inside a `UI.LineItem` collection — enough to
/// populate a Fiori-style default `$select` and label the column.
#[derive(Debug, Clone, Serialize)]
pub struct LineItemField {
    /// The `Value Path="..."` — the property path the column displays.
    pub value_path: String,
    /// Optional static `Label String="..."` override. When absent the
    /// frontend should fall back to the referenced property's own label.
    pub label: Option<String>,
    /// Optional `Importance` — one of "High", "Medium", "Low" (enum
    /// member on `UI.ImportanceType`). Stored as-is for the UI to style.
    pub importance: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Property {
    pub name: String,
    pub edm_type: String,
    pub nullable: bool,
    pub max_length: Option<u32>,
    pub label: Option<String>,
    /// Path of a sibling property that holds this property's
    /// human-readable description. Source: `Common.Text` (V4) or
    /// `sap:text` (V2). Example: `MaterialID.text_path = Some("MaterialDescription")`.
    pub text_path: Option<String>,
    /// Path of a sibling property that holds the unit of measure for this
    /// amount/quantity. Source: `Measures.Unit` (V4) or `sap:unit` (V2).
    /// Example: `NetWeight.unit_path = Some("WeightUnit")`.
    pub unit_path: Option<String>,
    /// Path of a sibling property that holds the ISO currency code for
    /// this monetary amount. Source: `Measures.ISOCurrency` (V4).
    /// Example: `NetAmount.iso_currency_path = Some("TransactionCurrency")`.
    pub iso_currency_path: Option<String>,
    /// Property-level flags. Each is `Some(bool)` only when the source
    /// metadata explicitly sets the flag; `None` means "use the OData
    /// default" (generally true for filterable/sortable/creatable/updatable).
    /// Sources: V2 `sap:filterable` / `sap:sortable` / `sap:creatable` /
    /// `sap:updatable` / `sap:required-in-filter`, plus V4
    /// `Capabilities.FilterRestrictions` / `SortRestrictions` /
    /// `InsertRestrictions` / `UpdateRestrictions` applied at the entity-set
    /// level. V4 restriction lists override the default-true interpretation:
    /// listed properties flip to `Some(false)` (or `Some(true)` for
    /// `RequiredProperties`).
    pub filterable: Option<bool>,
    pub sortable: Option<bool>,
    pub creatable: Option<bool>,
    pub updatable: Option<bool>,
    pub required_in_filter: Option<bool>,
    /// Parsed `UI.Criticality` annotation — either a fixed level
    /// (0 = Neutral, 1 = Negative, 2 = Critical, 3 = Positive,
    /// 5 = Information per OData spec) or a path to a sibling property
    /// whose runtime value supplies the level.
    pub criticality: Option<Criticality>,
    /// The "default" value help for this property — the first parsed
    /// `Common.ValueList` annotation (no-qualifier preferred, then
    /// qualified in source order). Retained for callers that just want
    /// one value help; the full set lives in `value_list_variants`.
    /// `None` when the service declares no inline value list.
    pub value_list: Option<ValueList>,
    /// All parsed `Common.ValueList` annotations for this property,
    /// including the one surfaced on `value_list`. When a property
    /// declares multiple qualified variants (e.g. "ByKey" vs
    /// "ByDescription"), consumers iterate this list to offer a
    /// variant picker in the F4 UI. Empty when there's no inline
    /// value list.
    pub value_list_variants: Vec<ValueList>,
    /// `Common.ValueListReferences` URLs captured verbatim. Each entry
    /// points at a separate Fiori value-help service whose `$metadata`
    /// contains the real `Common.ValueList` mapping. Empty when the
    /// property uses inline `ValueList` (see `value_list`) or no value
    /// help at all. Relative URLs are left unresolved — consumers
    /// resolve them against the current service's base URL.
    pub value_list_references: Vec<String>,
    /// `true` when `Common.ValueListWithFixedValues` is present on this
    /// property. Signals "the set of valid values is small and stable"
    /// (Fiori typically renders this as a dropdown). The annotation
    /// carries no mapping of its own, so it's a hint for the UI only.
    pub value_list_fixed: bool,
    /// How an ID column and its `Common.Text` companion should be
    /// displayed together. Source: `UI.TextArrangement` (per-property
    /// when nested inside `Common.Text`, or entity-type-level as the
    /// default for every text-bearing property in the type). `None`
    /// means "no explicit arrangement" — callers default to
    /// `TextFirst` per Fiori convention.
    pub text_arrangement: Option<TextArrangement>,
    /// `Common.FieldControl` — write/display control signal. Can be a
    /// fixed enum value (`Mandatory` / `Optional` / `ReadOnly` /
    /// `Inapplicable` / `Hidden`) or a path to a sibling property whose
    /// runtime value supplies the control code (1/3/7 = Read-only /
    /// Optional / Mandatory, per the SAP vocabulary numeric codes).
    pub field_control: Option<FieldControl>,
    /// `UI.Hidden` marker — Fiori should never show this property in
    /// any list/detail UI. We still render it in the describe panel
    /// (for transparency) but flag it clearly.
    pub hidden: bool,
    /// `UI.HiddenFilter` marker — property can be shown but must not be
    /// offered as a filter control. Signals "server knows about this
    /// but Fiori hides it from the filter bar".
    pub hidden_filter: bool,
    /// V2 `sap:display-format` attribute — presentation hint like
    /// `Date`, `NonNegative`, `UpperCase`. Stored verbatim (lowercased
    /// or not — SAP emits mixed case historically). Mainly useful for
    /// the describe overlay and future results-grid formatting.
    pub display_format: Option<String>,
    /// V2 `sap:value-list` attribute — marker that the property has a
    /// value help in this service (typical values: `"standard"` or
    /// `"fixed-values"`). V2 doesn't carry a mapping record inside
    /// `$metadata` (unlike V4's `Common.ValueList`), so this is hint-
    /// only: there's no picker target to open. Stored verbatim.
    pub sap_value_list: Option<String>,
    /// `Common.SemanticObject` — the name of a Fiori cross-app
    /// navigation target (e.g. `"Product"`, `"Customer"`). When
    /// present, clicking the property in Fiori would intent-navigate
    /// to another app. For the explorer this is a hint pill; future
    /// work could wire it to a "open in Fiori launchpad" action.
    pub semantic_object: Option<String>,
    /// `Common.Masked` marker — the property's value is sensitive
    /// (PII, secrets). Fiori renders it masked; the explorer should
    /// surface a warning so the user thinks twice before sharing
    /// screenshots or logs containing the column.
    pub masked: bool,
}

/// `Common.FieldControl` value. Fixed numeric codes map to the SAP
/// vocabulary's enum (7 = Mandatory, 3 = Optional, 1 = ReadOnly,
/// 0 = Inapplicable, 5 = Hidden); anything else parses as `Path`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
pub enum FieldControl {
    Mandatory,
    Optional,
    ReadOnly,
    Inapplicable,
    Hidden,
    /// Dynamic control — path to a sibling property whose runtime
    /// value (one of the numeric codes above) drives the state.
    Path(String),
}

/// `UI.TextArrangement` — how a coded-value column combines with its
/// `Common.Text` description when rendered.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TextArrangement {
    /// Description first, then the code in parens — "English (EN)".
    TextFirst,
    /// Code first, then the description in parens — "EN (English)".
    TextLast,
    /// Keep the two columns separate — no combination.
    TextSeparate,
    /// Description only; hide the code.
    TextOnly,
}

/// `UI.Criticality` value. Absent means "no criticality annotation",
/// not "neutral".
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
pub enum Criticality {
    Fixed(u8),
    Path(String),
}

/// Parsed `Common.ValueList` record — the F4/value-help declaration
/// for a property. Enough to drive a picker: where to fetch values
/// from, whether `$search` is available, and how picked rows map
/// back to the local entity's properties.
#[derive(Debug, Clone, Serialize)]
pub struct ValueList {
    /// Optional `Qualifier` on the annotation — distinguishes multiple
    /// value helps on the same property (e.g. `Qualifier="ByKey"` vs
    /// `Qualifier="ByDescription"`). `None` for the default variant.
    pub qualifier: Option<String>,
    /// Name of the entity set that serves as the value-help source.
    /// Path is relative to the service root — callers append it to
    /// the service URL directly.
    pub collection_path: String,
    /// Optional human-readable label for the value help (e.g.
    /// "Warehouse Value Help"). Falls back to `None` if unset.
    pub label: Option<String>,
    /// Whether the value-help collection accepts `$search`.
    /// `None` means the service didn't say; callers can choose to
    /// offer a search box speculatively.
    pub search_supported: Option<bool>,
    /// Parameter mapping between the annotated property's entity and
    /// the value-help entity. In source order.
    pub parameters: Vec<ValueListParameter>,
}

/// One entry inside `Common.ValueList.Parameters`. The kind determines
/// how the picker uses the mapping on open and on pick; `local_property`
/// is populated for In/Out/InOut (it's the column on the local entity),
/// and `constant` is populated only for `Constant`.
#[derive(Debug, Clone, Serialize)]
pub struct ValueListParameter {
    pub kind: ValueListParameterKind,
    /// Property name on the annotated entity that this parameter
    /// reads from or writes to. `None` for `DisplayOnly` (no local
    /// binding) and `Constant` (constant feeds the VL directly).
    pub local_property: Option<String>,
    /// Property name on the value-list entity.
    pub value_list_property: String,
    /// For `Constant` kind only: the literal value sent as a filter
    /// against `value_list_property`.
    pub constant: Option<String>,
}

/// The five standard `Common.ValueListParameter*` variants. Unknown
/// subtypes fall through parsing — we don't want to invent a kind we
/// can't faithfully drive.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ValueListParameterKind {
    In,
    Out,
    InOut,
    DisplayOnly,
    Constant,
}

/// Parsed `UI.HeaderInfo` record. All fields are optional — services
/// often populate only a subset.
#[derive(Debug, Clone, Serialize, Default)]
pub struct HeaderInfo {
    /// Singular human-readable entity name (e.g. "Warehouse Order").
    pub type_name: Option<String>,
    /// Plural form (e.g. "Warehouse Orders").
    pub type_name_plural: Option<String>,
    /// Path to the property used as the object's title at runtime
    /// (e.g. the `OrderNumber` on a `WarehouseOrder`).
    pub title_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NavigationProperty {
    pub name: String,
    /// V4: the target entity type (e.g., "Namespace.TypeName" or "Collection(...)").
    /// V2: empty — use `relationship` instead.
    pub target_type: String,
    /// V4: the partner nav property on the target side. V2: empty.
    pub partner: String,
    /// V2: the association name. V4: empty.
    pub relationship: String,
    /// V2: FromRole. V4: empty.
    pub from_role: String,
    /// V2: ToRole. V4: empty.
    pub to_role: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Association {
    pub name: String,
    pub ends: Vec<AssociationEnd>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssociationEnd {
    pub entity_type: String,
    pub multiplicity: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EntitySet {
    pub name: String,
    pub entity_type: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionImport {
    pub name: String,
    pub http_method: String,
    pub return_type: Option<String>,
    pub parameters: Vec<FunctionParameter>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionParameter {
    pub name: String,
    pub edm_type: String,
    pub mode: String,
}

impl ServiceMetadata {
    /// Find an entity type by name.
    pub fn find_entity_type(&self, name: &str) -> Option<&EntityType> {
        self.entity_types.iter().find(|et| et.name == name)
    }

    /// Find an entity set by name.
    pub fn find_entity_set(&self, name: &str) -> Option<&EntitySet> {
        self.entity_sets.iter().find(|es| es.name == name)
    }

    /// Get the entity type for a given entity set name.
    pub fn entity_type_for_set(&self, set_name: &str) -> Option<&EntityType> {
        let es = self.find_entity_set(set_name)?;
        let type_name = extract_type_name(&es.entity_type);
        self.find_entity_type(type_name)
    }

    /// Get navigation targets: (nav property name, target entity type name, multiplicity).
    pub fn nav_targets(&self, entity_type: &EntityType) -> Vec<(String, String, String)> {
        match self.version {
            ODataVersion::V4 => self.nav_targets_v4(entity_type),
            ODataVersion::V2 => self.nav_targets_v2(entity_type),
        }
    }

    fn nav_targets_v2(&self, entity_type: &EntityType) -> Vec<(String, String, String)> {
        entity_type
            .nav_properties
            .iter()
            .filter_map(|nav| {
                let assoc_name = extract_type_name(&nav.relationship);
                let assoc = self.associations.iter().find(|a| a.name == assoc_name)?;
                let target_end = assoc.ends.iter().find(|e| e.role == nav.to_role)?;
                let target_type = extract_type_name(&target_end.entity_type);
                Some((
                    nav.name.clone(),
                    target_type.to_string(),
                    target_end.multiplicity.clone(),
                ))
            })
            .collect()
    }

    fn nav_targets_v4(&self, entity_type: &EntityType) -> Vec<(String, String, String)> {
        entity_type
            .nav_properties
            .iter()
            .map(|nav| {
                let (type_str, multiplicity) = parse_v4_nav_type(&nav.target_type);
                let target_type = extract_type_name(type_str);
                (nav.name.clone(), target_type.to_string(), multiplicity)
            })
            .collect()
    }

    /// Count annotations grouped by vocabulary namespace. Cheap to call —
    /// it's an O(N) pass over the already-parsed `annotations` field.
    pub fn annotation_summary(&self) -> AnnotationSummary {
        let mut by_namespace: BTreeMap<String, usize> = BTreeMap::new();
        for ann in &self.annotations {
            *by_namespace.entry(ann.namespace.clone()).or_insert(0) += 1;
        }
        AnnotationSummary {
            total: self.annotations.len(),
            by_namespace,
        }
    }
}

/// Extract the simple type name from a qualified name (e.g., "Namespace.TypeName" → "TypeName").
pub(super) fn extract_type_name(qualified: &str) -> &str {
    qualified.rsplit('.').next().unwrap_or(qualified)
}

/// Parse V4 nav property Type: "Collection(Ns.Type)" → ("Ns.Type", "*") or "Ns.Type" → ("Ns.Type", "1").
pub(super) fn parse_v4_nav_type(type_str: &str) -> (&str, String) {
    if let Some(inner) = type_str
        .strip_prefix("Collection(")
        .and_then(|s| s.strip_suffix(')'))
    {
        (inner, "*".to_string())
    } else {
        (type_str, "1".to_string())
    }
}
