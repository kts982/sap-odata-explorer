//! Malicious-content boundary tests.
//!
//! SAP `$metadata` is untrusted input. A SAP system (compromised or
//! misconfigured) could declare entity names, property names, labels,
//! HeaderInfo titles, semantic-object names, qualifiers, or annotation
//! string values that look like HTML / JS / SQL injection payloads.
//!
//! The project's escaping boundary is:
//!
//!   parser keeps data intact  ─→  renderer escapes
//!
//! These tests pin down the *parser* half: `parse_metadata` must surface
//! malicious-shaped strings byte-for-byte without any sanitization. The
//! renderer half — `escapeHtml` / `safeHtml` / `raw()` — is exercised in
//! `scripts/test-safe-html.mjs` (and observed everywhere in the desktop
//! app via `lint-innerhtml.mjs`).
//!
//! If a future "helpful" change ever made the parser strip or re-encode
//! these strings, two things break: (1) the round-trip back to a SAP
//! system would lose data, and (2) the `safeHtml` contract would no
//! longer be load-bearing — the renderer would silently rely on the
//! parser having sanitized first, which is exactly the kind of
//! defence-in-depth gap that XSS bugs slip through.

use sap_odata_core::metadata::parse_metadata;

/// Embed common XSS / HTML-shaped payloads in every parser-surfaced
/// XML attribute slot we can reach from `$metadata`. Decoded by the XML
/// parser to the raw forms — exactly what a SAP system would push down
/// to clients on the wire.
const MALICIOUS_METADATA: &str = r##"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="Order&lt;img&gt;Type">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="Name&amp;Hostile" Type="Edm.String"/>
        <Property Name="Warehouse" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container">
        <EntitySet Name="Orders" EntityType="n.Order&lt;img&gt;Type"/>
      </EntityContainer>
      <Annotations Target="SAP__self.Order&lt;img&gt;Type">
        <Annotation Term="SAP__UI.HeaderInfo">
          <Record>
            <PropertyValue Property="TypeName" String="&lt;img src=x onerror=alert(1)&gt;"/>
            <PropertyValue Property="TypeNamePlural" String="&quot;Quoted &amp;ent;&quot;"/>
            <PropertyValue Property="Title">
              <Record>
                <PropertyValue Property="Value" Path="Name&amp;Hostile"/>
              </Record>
            </PropertyValue>
          </Record>
        </Annotation>
        <Annotation Term="SAP__UI.LineItem" Qualifier="evil&lt;script&gt;">
          <Collection>
            <Record Type="UI.DataField">
              <PropertyValue Property="Value" Path="ID"/>
              <PropertyValue Property="Label" String="&lt;b&gt;injected&lt;/b&gt;"/>
            </Record>
          </Collection>
        </Annotation>
      </Annotations>
      <Annotations Target="SAP__self.Order&lt;img&gt;Type/Warehouse">
        <Annotation Term="SAP__common.Label" String="javascript:alert(&quot;hi&quot;)"/>
        <Annotation Term="SAP__common.SemanticObject" String="&lt;svg/onload=alert(1)&gt;"/>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"##;

#[test]
fn parser_preserves_malicious_entity_and_property_names() {
    let meta = parse_metadata(MALICIOUS_METADATA).unwrap();
    // Entity name carries the literal `<img>` tag — the parser never
    // strips angle brackets. Renderer is responsible for escaping.
    let et = meta
        .find_entity_type("Order<img>Type")
        .expect("entity type with literal angle brackets must round-trip");
    assert_eq!(et.name, "Order<img>Type");

    // `&` in a property name comes through as a single `&` — XML decode
    // turns `&amp;` into `&`. Renderer must re-escape on output.
    let hostile = et
        .properties
        .iter()
        .find(|p| p.name == "Name&Hostile")
        .expect("property with `&` in its name must round-trip");
    assert_eq!(hostile.name, "Name&Hostile");
    // No silent sanitization that would replace `&` with `&amp;`.
    assert!(!hostile.name.contains("&amp;"));
}

#[test]
fn parser_preserves_malicious_header_info_strings() {
    let meta = parse_metadata(MALICIOUS_METADATA).unwrap();
    let et = meta.find_entity_type("Order<img>Type").unwrap();
    let hi = et
        .header_info
        .as_ref()
        .expect("HeaderInfo should parse despite hostile string contents");

    // Full XSS payload survives intact in TypeName — the renderer's
    // job is to neutralise it, not the parser's.
    assert_eq!(
        hi.type_name.as_deref(),
        Some("<img src=x onerror=alert(1)>")
    );
    // Embedded quotes + entities survive: `&quot;` → `"`, `&amp;ent;` → `&ent;`.
    assert_eq!(hi.type_name_plural.as_deref(), Some("\"Quoted &ent;\""));
    // Title path keeps the literal `&` in the property reference.
    assert_eq!(hi.title_path.as_deref(), Some("Name&Hostile"));
}

#[test]
fn parser_preserves_malicious_label_and_semantic_object() {
    let meta = parse_metadata(MALICIOUS_METADATA).unwrap();
    let et = meta.find_entity_type("Order<img>Type").unwrap();
    let warehouse = et
        .properties
        .iter()
        .find(|p| p.name == "Warehouse")
        .unwrap();

    // `javascript:` URI scheme is preserved verbatim in the label —
    // refusing to surface it at the parser layer would lose data
    // SAP intentionally pushed; the renderer must escape it instead.
    assert_eq!(warehouse.label.as_deref(), Some("javascript:alert(\"hi\")"));
    assert_eq!(
        warehouse.semantic_object.as_deref(),
        Some("<svg/onload=alert(1)>")
    );
}

#[test]
fn parser_preserves_malicious_line_item_label_and_qualifier() {
    let meta = parse_metadata(MALICIOUS_METADATA).unwrap();
    let et = meta.find_entity_type("Order<img>Type").unwrap();
    assert!(
        !et.line_item.is_empty(),
        "LineItem should parse even when its qualifier contains markup"
    );
    let f = &et.line_item[0];
    // Static label override carries the literal `<b>` tag.
    assert_eq!(f.label.as_deref(), Some("<b>injected</b>"));

    // Raw annotation list captures the full hostile qualifier verbatim.
    let line_item_annotation = meta
        .annotations
        .iter()
        .find(|a| a.term.ends_with("LineItem"))
        .expect("LineItem annotation should be in the raw list");
    assert_eq!(
        line_item_annotation.qualifier.as_deref(),
        Some("evil<script>")
    );
}
