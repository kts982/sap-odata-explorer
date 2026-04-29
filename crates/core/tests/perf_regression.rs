//! Perf-regression test for the metadata parser.
//!
//! Generates a multi-megabyte synthetic V4 CSDL with many entity types and
//! annotations, parses it through the network layer, and asserts the parse
//! completes inside a budget. Designed to catch accidental quadratic-time
//! regressions (e.g. an `O(n²)` annotation lookup, a string-search-by-find
//! loop). The fixture is generated in-test rather than committed because
//! the file would be ~3 MB and bloats the repo for no reason.
//!
//! `#[ignore]` because it's not a correctness test — it spends real CPU
//! time and skews "did my change break anything?" feedback. Run it
//! explicitly when worried about parser throughput:
//!
//!     cargo test -p sap-odata-core --test perf_regression -- --include-ignored

mod common;

use std::time::Instant;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SERVICE_PATH: &str = "/sap/opu/odata4/perf/SrvD";

/// Generate a synthetic V4 CSDL with `n` entity types, each with several
/// properties and annotation blocks. Total size scales linearly with `n`.
fn generate_large_metadata(n: usize) -> String {
    let mut out = String::with_capacity(n * 3_000);
    out.push_str(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" Version="4.0">
  <edmx:DataServices>
    <Schema xmlns="http://docs.oasis-open.org/odata/ns/edm" Namespace="PerfService">
"#,
    );

    // Entity types — keep the per-entity payload representative of real
    // SAP services: ~8 properties, a key, a couple of nav-property
    // declarations would be nice but they need targets so we stick to
    // scalar-only entities to keep the generator self-contained.
    for i in 0..n {
        out.push_str(&format!(
            r#"      <EntityType Name="Entity{i}">
        <Key><PropertyRef Name="Id"/></Key>
        <Property Name="Id" Type="Edm.String" Nullable="false" MaxLength="20"/>
        <Property Name="Name" Type="Edm.String" MaxLength="60"/>
        <Property Name="Description" Type="Edm.String" MaxLength="240"/>
        <Property Name="Status" Type="Edm.Int32"/>
        <Property Name="Quantity" Type="Edm.Decimal"/>
        <Property Name="Unit" Type="Edm.String" MaxLength="3"/>
        <Property Name="Currency" Type="Edm.String" MaxLength="5"/>
        <Property Name="Amount" Type="Edm.Decimal"/>
        <Property Name="LastChangedAt" Type="Edm.DateTimeOffset"/>
      </EntityType>
"#,
        ));
    }

    // Container with one entity set per type.
    out.push_str(
        r#"      <EntityContainer Name="DefaultContainer">
"#,
    );
    for i in 0..n {
        out.push_str(&format!(
            r#"        <EntitySet Name="Entity{i}Set" EntityType="PerfService.Entity{i}"/>
"#,
        ));
    }
    out.push_str("      </EntityContainer>\n");

    // Annotation blocks — one per entity type, with the annotations real
    // services typically declare. This is the bulk of `$metadata` size on
    // production systems.
    for i in 0..n {
        out.push_str(&format!(
            r#"      <Annotations Target="PerfService.Entity{i}">
        <Annotation Term="UI.HeaderInfo">
          <Record>
            <PropertyValue Property="TypeName" String="Entity{i}"/>
            <PropertyValue Property="TypeNamePlural" String="Entity{i}s"/>
            <PropertyValue Property="Title">
              <Record><PropertyValue Property="Value" Path="Name"/></Record>
            </PropertyValue>
          </Record>
        </Annotation>
        <Annotation Term="UI.LineItem">
          <Collection>
            <Record Type="UI.DataField"><PropertyValue Property="Value" Path="Id"/></Record>
            <Record Type="UI.DataField"><PropertyValue Property="Value" Path="Name"/></Record>
            <Record Type="UI.DataField"><PropertyValue Property="Value" Path="Status"/></Record>
          </Collection>
        </Annotation>
        <Annotation Term="UI.SelectionFields">
          <Collection>
            <PropertyPath>Id</PropertyPath>
            <PropertyPath>Status</PropertyPath>
          </Collection>
        </Annotation>
      </Annotations>
      <Annotations Target="PerfService.Entity{i}/Quantity">
        <Annotation Term="Measures.Unit" Path="Unit"/>
      </Annotations>
      <Annotations Target="PerfService.Entity{i}/Amount">
        <Annotation Term="Measures.ISOCurrency" Path="Currency"/>
      </Annotations>
"#,
        ));
    }

    out.push_str(
        r#"    </Schema>
  </edmx:DataServices>
</edmx:Edmx>
"#,
    );

    out
}

#[tokio::test]
#[ignore = "perf regression — runs only with --include-ignored"]
async fn parses_large_metadata_within_budget() {
    // 1500 entity types × ~2 KB per (props + annotations) ≈ 3 MB, large
    // enough that quadratic-time regressions blow the budget but small
    // enough to generate in <100ms.
    const ENTITY_COUNT: usize = 1500;
    // Two-second budget gives headroom for a slow CI runner. On a
    // developer laptop this typically completes in ~50–150ms.
    const BUDGET_MS: u128 = 2000;

    let xml = generate_large_metadata(ENTITY_COUNT);
    let size_kb = xml.len() / 1024;
    eprintln!("perf: generated metadata = {size_kb} KB ({ENTITY_COUNT} entity types)");

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(SERVICE_PATH))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{SERVICE_PATH}/$metadata")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "application/xml")
                .set_body_string(xml.clone()),
        )
        .mount(&server)
        .await;

    let client = common::basic_client(&server);
    let started = Instant::now();
    let meta = client
        .fetch_metadata(SERVICE_PATH)
        .await
        .expect("large metadata should parse");
    let elapsed_ms = started.elapsed().as_millis();
    eprintln!(
        "perf: fetched + parsed {} entity types in {} ms",
        meta.entity_types.len(),
        elapsed_ms
    );

    assert_eq!(meta.entity_types.len(), ENTITY_COUNT);
    assert_eq!(meta.entity_sets.len(), ENTITY_COUNT);
    assert!(
        elapsed_ms < BUDGET_MS,
        "parse + fetch took {elapsed_ms} ms, budget is {BUDGET_MS} ms — \
         likely a quadratic-time regression in the parser. \
         Generated metadata size was {size_kb} KB."
    );
}
