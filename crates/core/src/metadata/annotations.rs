//! Typed-annotation pass over parsed entity types.
//!
//! After `parse_metadata` walks the raw XML and produces a list of
//! `EntityType` / `Property` / `EntitySet` records, this module decorates
//! them with typed accessors derived from `<Annotations Target="...">`
//! blocks: `UI.HeaderInfo`, `UI.LineItem`, `UI.SelectionFields`,
//! `UI.PresentationVariant`, `UI.SelectionVariant`, `Common.Text`,
//! `Common.Label`, `Common.FieldControl`, `Common.SemanticKey`,
//! `Common.SemanticObject`, `Common.Masked`, `Common.ValueList`,
//! `Common.ValueListReferences`, `Common.ValueListWithFixedValues`,
//! `Measures.Unit`, `Measures.ISOCurrency`, `UI.Hidden`, `UI.HiddenFilter`,
//! `UI.Criticality`, `UI.TextArrangement`, plus entity-set–targeted
//! `Capabilities.{Filter,Sort,Insert,Update,Search,Count,Top,Skip,Expand}Restrictions`.
//!
//! Each new vocabulary term lives in a single `else if` branch in
//! `apply_v4_typed_annotations`; record parsing is delegated to small
//! `parse_X_record` helpers that take a `roxmltree::Node` and return the
//! typed shape. Entry point is `pub(super) fn apply_v4_typed_annotations`,
//! invoked from `parse_metadata` after entity-set parsing.
//!
//! Public surface: `parse_value_list_mapping_xml(xml, local_property)` —
//! used by reference resolution in the desktop app to parse a `Common
//! .ValueListMapping` from a separate F4 service's `$metadata` once the
//! reference URL has been fetched. Re-exported from the module root.

use super::model::extract_type_name;
use super::model::*;
use super::{children_by_tag, strip_alias_prefix};

/// Second pass over `<Annotations Target>` blocks that populates typed
/// accessors on the parsed entity types. Handles `UI.HeaderInfo`,
/// `Common.Text`, `UI.Criticality`, `Measures.Unit`, `Measures.ISOCurrency`
/// (all targeted at entity types / properties), and the entity-set–targeted
/// `Capabilities.FilterRestrictions` / `SortRestrictions` /
/// `InsertRestrictions` / `UpdateRestrictions` property lists.
/// Each additional vocabulary term gets its own branch here, keeping
/// complex Record/Collection parsing in one place instead of spilling
/// into callers.
pub(super) fn apply_v4_typed_annotations(
    entity_types: &mut [EntityType],
    entity_sets: &[EntitySet],
    schema: &roxmltree::Node,
    alias: &str,
) {
    for annots_node in children_by_tag(schema, "Annotations") {
        let raw_target = annots_node.attribute("Target").unwrap_or("");
        let target = strip_alias_prefix(raw_target, alias);

        for annot in children_by_tag(&annots_node, "Annotation") {
            let term = annot.attribute("Term").unwrap_or("");
            // Normalize: strip the SAP__ alias prefix and lowercase so the
            // match works for `UI.HeaderInfo`, `SAP__UI.HeaderInfo`,
            // `com.sap.vocabularies.UI.v1.HeaderInfo`, etc.
            let lower = term.trim_start_matches("SAP__").to_ascii_lowercase();

            if lower.ends_with(".headerinfo") && !target.contains('/') {
                if let Some(et) = entity_types.iter_mut().find(|e| e.name == target)
                    && let Some(info) = parse_header_info_record(&annot)
                {
                    et.header_info = Some(info);
                }
            } else if lower.ends_with(".text") && !lower.ends_with(".textarrangement") {
                if let Some((et_name, prop_name)) = target.split_once('/')
                    && let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name)
                    && let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name)
                {
                    if prop.text_path.is_none() {
                        prop.text_path = annot.attribute("Path").map(String::from);
                    }
                    // A `UI.TextArrangement` nested inside the
                    // `Common.Text` annotation is the per-property
                    // override — takes priority over any type-level
                    // default set later.
                    for nested in children_by_tag(&annot, "Annotation") {
                        let n_term = nested.attribute("Term").unwrap_or("");
                        let n_lower = n_term.trim_start_matches("SAP__").to_ascii_lowercase();
                        if n_lower.ends_with(".textarrangement")
                            && let Some(ta) = parse_text_arrangement(&nested)
                        {
                            prop.text_arrangement = Some(ta);
                        }
                    }
                }
            } else if lower.ends_with(".textarrangement") {
                // Entity-type-level default: apply to every property that
                // already has a `text_path` and no per-property override.
                // (We parse annotations in file order, and `Common.Text`
                // is typically declared *before* a standalone
                // `UI.TextArrangement` on the type — but we guard against
                // either order.)
                if !target.contains('/')
                    && let Some(et) = entity_types.iter_mut().find(|e| e.name == target)
                    && let Some(ta) = parse_text_arrangement(&annot)
                {
                    for prop in et.properties.iter_mut() {
                        if prop.text_arrangement.is_none() {
                            prop.text_arrangement = Some(ta);
                        }
                    }
                }
            } else if lower.ends_with(".semantickey") && !target.contains('/') {
                // Common.SemanticKey → Collection<PropertyPath> at the
                // entity-type level. Captures the business key (often
                // distinct from the technical primary key, e.g. Product
                // instead of a UUID).
                if let Some(et) = entity_types.iter_mut().find(|e| e.name == target) {
                    for coll in children_by_tag(&annot, "Collection") {
                        for pp in children_by_tag(&coll, "PropertyPath") {
                            if let Some(text) = pp.text() {
                                let trimmed = text.trim();
                                if !trimmed.is_empty()
                                    && !et.semantic_keys.iter().any(|x| x == trimmed)
                                {
                                    et.semantic_keys.push(trimmed.to_string());
                                }
                            }
                        }
                    }
                }
            } else if lower.ends_with(".semanticobject") {
                if let Some((et_name, prop_name)) = target.split_once('/')
                    && let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name)
                    && let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name)
                    && prop.semantic_object.is_none()
                {
                    prop.semantic_object = annot.attribute("String").map(String::from);
                }
            } else if lower.ends_with(".masked") {
                // Marker-only annotation; presence is enough. SAP may
                // emit it with an `EnumMember` variant for "masked with
                // stars vs dots" etc. — we collapse to a single boolean
                // since the explorer just needs a visual warning.
                if let Some((et_name, prop_name)) = target.split_once('/')
                    && let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name)
                    && let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name)
                {
                    prop.masked = true;
                }
            } else if lower.ends_with(".fieldcontrol") {
                if let Some((et_name, prop_name)) = target.split_once('/')
                    && let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name)
                    && let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name)
                    && let Some(fc) = parse_field_control(&annot)
                {
                    prop.field_control = Some(fc);
                }
            } else if lower.ends_with(".hidden") && !lower.ends_with(".hiddenfilter") {
                // UI.Hidden can be marker-only (`<Annotation .../>`) or
                // carry a `Bool` / `Path` — Fiori convention treats a
                // missing value as `true`. Path variants are
                // runtime-evaluated; we don't resolve them per row, so
                // a Path here still flips the static marker on.
                if let Some((et_name, prop_name)) = target.split_once('/')
                    && let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name)
                    && let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name)
                {
                    let explicit = annot.attribute("Bool").map(|v| v == "true");
                    prop.hidden = explicit.unwrap_or(true);
                }
            } else if lower.ends_with(".hiddenfilter") {
                if let Some((et_name, prop_name)) = target.split_once('/')
                    && let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name)
                    && let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name)
                {
                    let explicit = annot.attribute("Bool").map(|v| v == "true");
                    prop.hidden_filter = explicit.unwrap_or(true);
                }
            } else if lower.ends_with(".criticality") {
                if let Some((et_name, prop_name)) = target.split_once('/')
                    && let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name)
                    && let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name)
                    && let Some(c) = parse_criticality(&annot)
                {
                    prop.criticality = Some(c);
                }
            } else if lower.ends_with(".unit") && !lower.ends_with(".isunit") {
                if let Some((et_name, prop_name)) = target.split_once('/')
                    && let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name)
                    && let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name)
                    && prop.unit_path.is_none()
                {
                    prop.unit_path = annot.attribute("Path").map(String::from);
                }
            } else if lower.ends_with(".isocurrency") {
                if let Some((et_name, prop_name)) = target.split_once('/')
                    && let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name)
                    && let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name)
                {
                    prop.iso_currency_path = annot.attribute("Path").map(String::from);
                }
            } else if lower.ends_with(".filterrestrictions")
                || lower.ends_with(".sortrestrictions")
                || lower.ends_with(".insertrestrictions")
                || lower.ends_with(".updaterestrictions")
            {
                // Entity-set–scoped capability restriction.
                // Target looks like "Container/EntitySetName".
                if let Some((_, set_name)) = target.split_once('/')
                    && let Some(type_ref) = entity_sets.iter().find(|s| s.name == set_name)
                {
                    let type_name = extract_type_name(&type_ref.entity_type).to_string();
                    if let Some(et) = entity_types.iter_mut().find(|t| t.name == type_name) {
                        apply_capability_restriction(&lower, &annot, &mut et.properties);
                    }
                }
            } else if lower.ends_with(".searchrestrictions")
                || lower.ends_with(".countrestrictions")
                || lower.ends_with(".expandrestrictions")
            {
                // Entity-set–scoped. These set flat Option<bool>s and/or
                // a nav-path list on the EntityType so the frontend
                // pre-flight validator can catch queries that will 500.
                if let Some((_, set_name)) = target.split_once('/')
                    && let Some(type_ref) = entity_sets.iter().find(|s| s.name == set_name)
                {
                    let type_name = extract_type_name(&type_ref.entity_type).to_string();
                    if let Some(et) = entity_types.iter_mut().find(|t| t.name == type_name) {
                        apply_entity_set_capability(&lower, &annot, et);
                    }
                }
            } else if lower.ends_with(".topsupported") || lower.ends_with(".skipsupported") {
                // Standalone `<Annotation ... Bool="false"/>` on an entity
                // set. Default is `true`, so only `false` is informative
                // (but we store whatever was declared for transparency).
                if let Some((_, set_name)) = target.split_once('/')
                    && let Some(type_ref) = entity_sets.iter().find(|s| s.name == set_name)
                {
                    let type_name = extract_type_name(&type_ref.entity_type).to_string();
                    if let Some(et) = entity_types.iter_mut().find(|t| t.name == type_name) {
                        let val = annot.attribute("Bool").map(|v| v == "true");
                        if lower.ends_with(".topsupported") {
                            et.top_supported = val;
                        } else {
                            et.skip_supported = val;
                        }
                    }
                }
            } else if lower.ends_with(".valuelist")
                && !lower.ends_with(".valuelistreferences")
                && !lower.ends_with(".valuelistmapping")
                && !lower.ends_with(".valuelistwithfixedvalues")
            {
                // Common.ValueList targets a specific property. We keep
                // every parsed variant — the no-qualifier one lands on
                // `value_list` (first-fill) for consumers that want a
                // single default; all are pushed into
                // `value_list_variants` so the picker can offer a
                // variant switcher. Dedupe by qualifier so a repeated
                // annotation on the same target doesn't double-count.
                if let Some((et_name, prop_name)) = target.split_once('/')
                    && let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name)
                    && let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name)
                    && let Some(mut vl) = parse_value_list_record(&annot)
                {
                    vl.qualifier = annot.attribute("Qualifier").map(String::from);
                    let already_have = prop
                        .value_list_variants
                        .iter()
                        .any(|existing| existing.qualifier == vl.qualifier);
                    if !already_have {
                        if prop.value_list.is_none() {
                            prop.value_list = Some(vl.clone());
                        }
                        prop.value_list_variants.push(vl);
                    }
                }
            } else if lower.ends_with(".valuelistreferences") {
                // Common.ValueListReferences carries a Collection of
                // String URLs that point at separate value-help services.
                // Each URL resolves (relative to the current service) to
                // an F4 service whose `$metadata` contains the real
                // `Common.ValueList` mapping. Capture all URLs so the
                // frontend can try multiple references if needed.
                if let Some((et_name, prop_name)) = target.split_once('/')
                    && let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name)
                    && let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name)
                {
                    for coll in children_by_tag(&annot, "Collection") {
                        for s in children_by_tag(&coll, "String") {
                            if let Some(text) = s.text() {
                                let trimmed = text.trim();
                                if !trimmed.is_empty()
                                    && !prop.value_list_references.iter().any(|u| u == trimmed)
                                {
                                    prop.value_list_references.push(trimmed.to_string());
                                }
                            }
                        }
                    }
                }
            } else if lower.ends_with(".valuelistwithfixedvalues") {
                // Marker-only annotation — flips a boolean on the
                // property. The term is almost always written as an
                // empty `<Annotation .../>`; we don't inspect any
                // attributes.
                if let Some((et_name, prop_name)) = target.split_once('/')
                    && let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name)
                    && let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name)
                {
                    prop.value_list_fixed = true;
                }
            } else if lower.ends_with(".selectionpresentationvariant") {
                // UI.SelectionPresentationVariant wraps a named view:
                // Text + ID + (SelectionVariant inline Record OR Path)
                // + (PresentationVariant inline Record OR Path). Path
                // references point at peer qualified annotations that
                // our standalone SV/PV dispatches already capture, so
                // we only inspect inline records here. Inline records
                // are unusual in SAP-CDS output but supported by the
                // vocabulary — this keeps services that emit them
                // (hand-written CDS, tests) working.
                let qualifier = annot.attribute("Qualifier").map(String::from);
                let record = match children_by_tag(&annot, "Record").into_iter().next() {
                    Some(r) => r,
                    None => continue,
                };
                let mut spv_text: Option<String> = None;
                let mut inline_sv: Option<roxmltree::Node> = None;
                let mut inline_pv: Option<roxmltree::Node> = None;
                for pv in children_by_tag(&record, "PropertyValue") {
                    match pv.attribute("Property") {
                        Some("Text") => spv_text = pv.attribute("String").map(String::from),
                        Some("SelectionVariant") if pv.attribute("Path").is_none() => {
                            inline_sv = children_by_tag(&pv, "Record").into_iter().next();
                        }
                        Some("PresentationVariant") if pv.attribute("Path").is_none() => {
                            inline_pv = children_by_tag(&pv, "Record").into_iter().next();
                        }
                        _ => {}
                    }
                }
                let type_name = if let Some((_, set_name)) = target.split_once('/') {
                    entity_sets
                        .iter()
                        .find(|s| s.name == set_name)
                        .map(|s| extract_type_name(&s.entity_type).to_string())
                } else {
                    Some(target.to_string())
                };
                if let Some(name) = type_name
                    && let Some(et) = entity_types.iter_mut().find(|t| t.name == name)
                {
                    // Inline SelectionVariant → materialize and add,
                    // deduped by qualifier against existing variants.
                    if let Some(sv_record) = inline_sv {
                        let already_have = qualifier.is_some()
                            && et
                                .selection_variants
                                .iter()
                                .any(|v| v.qualifier == qualifier);
                        if !already_have {
                            let fake_annot = sv_record.parent().unwrap_or(sv_record);
                            // Build a variant from the inline record
                            // directly. We can't reuse
                            // parse_selection_variant_record since
                            // it expects the outer <Annotation> as
                            // input — just walk the Record here.
                            let mut variant = SelectionVariant {
                                qualifier: qualifier.clone(),
                                text: spv_text.clone(),
                                parameters: Vec::new(),
                                select_options: Vec::new(),
                            };
                            for pv in children_by_tag(&sv_record, "PropertyValue") {
                                match pv.attribute("Property") {
                                    Some("Text") if variant.text.is_none() => {
                                        variant.text = pv.attribute("String").map(String::from);
                                    }
                                    Some("Parameters") => {
                                        for coll in children_by_tag(&pv, "Collection") {
                                            for rec in children_by_tag(&coll, "Record") {
                                                if let Some(p) = parse_selection_parameter(&rec) {
                                                    variant.parameters.push(p);
                                                }
                                            }
                                        }
                                    }
                                    Some("SelectOptions") => {
                                        for coll in children_by_tag(&pv, "Collection") {
                                            for rec in children_by_tag(&coll, "Record") {
                                                if let Some(o) = parse_select_option(&rec) {
                                                    variant.select_options.push(o);
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            if qualifier.is_none() {
                                et.selection_variants.insert(0, variant);
                            } else {
                                et.selection_variants.push(variant);
                            }
                            let _ = fake_annot; // silence unused-hint lints
                        }
                    }
                    // Inline PresentationVariant → reuse existing
                    // parser on the outer-ish record by synthesizing
                    // an annotation wrapping it. Simpler path: walk
                    // the record directly since we control the shape.
                    if let Some(pv_record) = inline_pv {
                        for pv in children_by_tag(&pv_record, "PropertyValue") {
                            match pv.attribute("Property") {
                                Some("RequestAtLeast") if et.request_at_least.is_empty() => {
                                    for coll in children_by_tag(&pv, "Collection") {
                                        for pp in children_by_tag(&coll, "PropertyPath") {
                                            if let Some(text) = pp.text() {
                                                let trimmed = text.trim();
                                                if !trimmed.is_empty()
                                                    && !et
                                                        .request_at_least
                                                        .iter()
                                                        .any(|x| x == trimmed)
                                                {
                                                    et.request_at_least.push(trimmed.to_string());
                                                }
                                            }
                                        }
                                    }
                                }
                                Some("SortOrder") if et.sort_order.is_empty() => {
                                    for coll in children_by_tag(&pv, "Collection") {
                                        for rec in children_by_tag(&coll, "Record") {
                                            if let Some(so) = parse_sort_order_record(&rec) {
                                                et.sort_order.push(so);
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            } else if lower.ends_with(".selectionvariant") {
                // UI.SelectionVariant = declared Fiori filter variant.
                // Multiple qualifiers are allowed (one per named variant),
                // so we push every match rather than keeping just the
                // first. Targets the entity type or the entity set.
                let variant_opt = parse_selection_variant_record(&annot);
                if let Some(mut variant) = variant_opt {
                    variant.qualifier = annot.attribute("Qualifier").map(String::from);
                    let type_name = if let Some((_, set_name)) = target.split_once('/') {
                        entity_sets
                            .iter()
                            .find(|s| s.name == set_name)
                            .map(|s| extract_type_name(&s.entity_type).to_string())
                    } else {
                        Some(target.to_string())
                    };
                    if let Some(name) = type_name
                        && let Some(et) = entity_types.iter_mut().find(|t| t.name == name)
                    {
                        // Default (no-qualifier) variant goes first so
                        // the frontend's "pick one" logic can grab it
                        // with .first(). Qualified variants preserve
                        // their parse order after.
                        if variant.qualifier.is_none() {
                            et.selection_variants.insert(0, variant);
                        } else {
                            et.selection_variants.push(variant);
                        }
                    }
                }
            } else if lower.ends_with(".presentationvariant") {
                // UI.PresentationVariant = Record with RequestAtLeast /
                // SortOrder / Visualizations / ... We lift the two
                // actionable pieces — `$select` augment paths and the
                // default `$orderby` order. Targets the entity type or
                // the entity set; resolve both.
                let (paths, sort) = parse_presentation_variant(&annot);
                if paths.is_empty() && sort.is_empty() {
                    continue;
                }
                let type_name = if let Some((_, set_name)) = target.split_once('/') {
                    entity_sets
                        .iter()
                        .find(|s| s.name == set_name)
                        .map(|s| extract_type_name(&s.entity_type).to_string())
                } else {
                    Some(target.to_string())
                };
                if let Some(name) = type_name
                    && let Some(et) = entity_types.iter_mut().find(|t| t.name == name)
                {
                    if !paths.is_empty() && et.request_at_least.is_empty() {
                        et.request_at_least = paths;
                    }
                    if !sort.is_empty() && et.sort_order.is_empty() {
                        et.sort_order = sort;
                    }
                }
            } else if lower.ends_with(".lineitem") {
                // UI.LineItem = Collection(UI.DataField). Mirrors
                // SelectionFields' target-resolution pattern: either
                // the entity set (Container/SetName) or the type name.
                let fields = parse_line_item_collection(&annot);
                if fields.is_empty() {
                    continue;
                }
                let type_name = if let Some((_, set_name)) = target.split_once('/') {
                    entity_sets
                        .iter()
                        .find(|s| s.name == set_name)
                        .map(|s| extract_type_name(&s.entity_type).to_string())
                } else {
                    Some(target.to_string())
                };
                if let Some(name) = type_name
                    && let Some(et) = entity_types.iter_mut().find(|t| t.name == name)
                {
                    // Prefer the no-qualifier (default) variant
                    // regardless of source order: fill when empty,
                    // and always overwrite with the unqualified
                    // one. A qualified variant never displaces an
                    // already-stored unqualified one. Mirrors how
                    // we handle UI.SelectionVariant.
                    let is_qualified = annot.attribute("Qualifier").is_some();
                    if et.line_item.is_empty() || !is_qualified {
                        et.line_item = fields;
                    }
                }
            } else if lower.ends_with(".selectionfields") {
                // UI.SelectionFields can target either the entity set
                // ("Container/SetName") or the entity type directly
                // ("TypeName"). SAP-generated services put it on the type;
                // hand-written ones often put it on the set. Handle both.
                let paths = parse_property_path_collection(&annot);
                if paths.is_empty() {
                    continue;
                }
                let type_name = if let Some((_, set_name)) = target.split_once('/') {
                    entity_sets
                        .iter()
                        .find(|s| s.name == set_name)
                        .map(|s| extract_type_name(&s.entity_type).to_string())
                } else {
                    Some(target.to_string())
                };
                if let Some(name) = type_name
                    && let Some(et) = entity_types.iter_mut().find(|t| t.name == name)
                {
                    // Same precedence rule as UI.LineItem: the
                    // unqualified variant wins regardless of
                    // source order.
                    let is_qualified = annot.attribute("Qualifier").is_some();
                    if et.selection_fields.is_empty() || !is_qualified {
                        et.selection_fields = paths;
                    }
                }
            }
        }
    }
}

/// Apply a single `Capabilities.*Restrictions` record to an entity type's
/// property list. Pulls out the collection-of-PropertyPath payloads for
/// the five lists we care about and flips the relevant `Option<bool>`
/// on each listed property. Unknown fields are ignored.
fn apply_capability_restriction(
    lower_term: &str,
    annot: &roxmltree::Node,
    properties: &mut [Property],
) {
    let record = match children_by_tag(annot, "Record").into_iter().next() {
        Some(r) => r,
        None => return,
    };
    for pv in children_by_tag(&record, "PropertyValue") {
        let prop_name = match pv.attribute("Property") {
            Some(n) => n,
            None => continue,
        };
        let paths = parse_property_path_collection(&pv);
        let (field_name, value) = match (lower_term, prop_name) {
            (t, "NonFilterableProperties") if t.ends_with(".filterrestrictions") => {
                ("filterable", Some(false))
            }
            (t, "RequiredProperties") if t.ends_with(".filterrestrictions") => {
                ("required_in_filter", Some(true))
            }
            (t, "NonSortableProperties") if t.ends_with(".sortrestrictions") => {
                ("sortable", Some(false))
            }
            (t, "NonInsertableProperties") if t.ends_with(".insertrestrictions") => {
                ("creatable", Some(false))
            }
            (t, "NonUpdatableProperties") if t.ends_with(".updaterestrictions") => {
                ("updatable", Some(false))
            }
            _ => continue,
        };
        for path in paths {
            if let Some(prop) = properties.iter_mut().find(|p| p.name == path) {
                match field_name {
                    "filterable" => prop.filterable = value,
                    "sortable" => prop.sortable = value,
                    "creatable" => prop.creatable = value,
                    "updatable" => prop.updatable = value,
                    "required_in_filter" => prop.required_in_filter = value,
                    _ => {}
                }
            }
        }
    }
}

/// Apply an entity-set–scoped capability annotation that lives on
/// the EntityType itself (not on individual properties).
/// Handles `Capabilities.SearchRestrictions`, `CountRestrictions`,
/// and `ExpandRestrictions`. The outer record's `Property` values are
/// the interesting payloads: `Searchable` / `Countable` / `Expandable`
/// booleans, and `NonExpandableProperties` as a NavigationPropertyPath
/// collection. Unknown fields are ignored so new Capability sub-terms
/// don't break the parser.
fn apply_entity_set_capability(lower_term: &str, annot: &roxmltree::Node, et: &mut EntityType) {
    let record = match children_by_tag(annot, "Record").into_iter().next() {
        Some(r) => r,
        None => return,
    };
    for pv in children_by_tag(&record, "PropertyValue") {
        let prop_name = match pv.attribute("Property") {
            Some(n) => n,
            None => continue,
        };
        match (lower_term, prop_name) {
            (t, "Searchable") if t.ends_with(".searchrestrictions") => {
                et.searchable = pv.attribute("Bool").map(|v| v == "true");
            }
            (t, "Countable") if t.ends_with(".countrestrictions") => {
                et.countable = pv.attribute("Bool").map(|v| v == "true");
            }
            (t, "Expandable") if t.ends_with(".expandrestrictions") => {
                et.expandable = pv.attribute("Bool").map(|v| v == "true");
            }
            (t, "NonExpandableProperties") if t.ends_with(".expandrestrictions") => {
                // NavigationPropertyPath collection — same shape as
                // PropertyPath. Reuse the existing helper rather than
                // duplicating the walk.
                for coll in children_by_tag(&pv, "Collection") {
                    for npp in children_by_tag(&coll, "NavigationPropertyPath") {
                        if let Some(text) = npp.text() {
                            let trimmed = text.trim();
                            if !trimmed.is_empty()
                                && !et.non_expandable_properties.iter().any(|x| x == trimmed)
                            {
                                et.non_expandable_properties.push(trimmed.to_string());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Parse the `Record` inside a `Common.ValueList` annotation. Returns
/// `None` if the record is missing, has no `CollectionPath`, or has an
/// empty `Parameters` collection — any of those make the value help
/// unusable for the picker. Skips parameter records whose Type we
/// don't recognize rather than inventing a mapping we can't drive.
fn parse_value_list_record(annot: &roxmltree::Node) -> Option<ValueList> {
    let record = children_by_tag(annot, "Record").into_iter().next()?;
    let mut collection_path: Option<String> = None;
    let mut label: Option<String> = None;
    let mut search_supported: Option<bool> = None;
    let mut parameters: Vec<ValueListParameter> = Vec::new();
    for pv in children_by_tag(&record, "PropertyValue") {
        match pv.attribute("Property") {
            Some("CollectionPath") => {
                collection_path = pv.attribute("String").map(String::from);
            }
            Some("Label") => {
                label = pv.attribute("String").map(String::from);
            }
            Some("SearchSupported") => {
                search_supported = pv.attribute("Bool").map(|v| v == "true");
            }
            Some("Parameters") => {
                for coll in children_by_tag(&pv, "Collection") {
                    for param_rec in children_by_tag(&coll, "Record") {
                        if let Some(p) = parse_value_list_parameter(&param_rec) {
                            parameters.push(p);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let collection_path = collection_path?;
    if parameters.is_empty() {
        return None;
    }
    Some(ValueList {
        qualifier: None,
        collection_path,
        label,
        search_supported,
        parameters,
    })
}

/// Parse one `<Record>` inside `Common.ValueList.Parameters`. Returns
/// `None` for unrecognized record types — skipping beats inventing.
fn parse_value_list_parameter(record: &roxmltree::Node) -> Option<ValueListParameter> {
    let type_attr = record.attribute("Type")?;
    // Accept SAP__-aliased, fully qualified, or bare names. The leaf is
    // the segment after the last dot.
    let normalized = type_attr.trim_start_matches("SAP__");
    let leaf = normalized.rsplit('.').next().unwrap_or(normalized);
    let kind = match leaf {
        "ValueListParameterIn" => ValueListParameterKind::In,
        "ValueListParameterOut" => ValueListParameterKind::Out,
        "ValueListParameterInOut" => ValueListParameterKind::InOut,
        "ValueListParameterDisplayOnly" => ValueListParameterKind::DisplayOnly,
        "ValueListParameterConstant" => ValueListParameterKind::Constant,
        _ => return None,
    };
    let mut local_property: Option<String> = None;
    let mut value_list_property: Option<String> = None;
    let mut constant: Option<String> = None;
    for pv in children_by_tag(record, "PropertyValue") {
        match pv.attribute("Property") {
            Some("LocalDataProperty") => {
                local_property = pv.attribute("PropertyPath").map(String::from);
            }
            Some("ValueListProperty") => {
                value_list_property = pv.attribute("String").map(String::from);
            }
            Some("Constant") => {
                // Spec allows several primitive-typed attrs; SAP most
                // often emits String=. Fall through to Int/Bool for the
                // rarer numeric/boolean constants.
                constant = pv
                    .attribute("String")
                    .or_else(|| pv.attribute("Int"))
                    .or_else(|| pv.attribute("Bool"))
                    .map(String::from);
            }
            _ => {}
        }
    }
    let value_list_property = value_list_property?;
    Some(ValueListParameter {
        kind,
        local_property,
        value_list_property,
        constant,
    })
}

/// Parse the `Record` inside a `UI.SelectionVariant` annotation into
/// a typed variant. Qualifier is stitched on by the caller (it's on
/// the outer `<Annotation Qualifier="...">`, not on the record). Returns
/// `None` only when the annotation has no inner `<Record>`.
fn parse_selection_variant_record(annot: &roxmltree::Node) -> Option<SelectionVariant> {
    let record = children_by_tag(annot, "Record").into_iter().next()?;
    let mut text: Option<String> = None;
    let mut parameters: Vec<SelectionParameter> = Vec::new();
    let mut select_options: Vec<SelectOption> = Vec::new();
    for pv in children_by_tag(&record, "PropertyValue") {
        match pv.attribute("Property") {
            Some("Text") => text = pv.attribute("String").map(String::from),
            Some("Parameters") => {
                for coll in children_by_tag(&pv, "Collection") {
                    for param_rec in children_by_tag(&coll, "Record") {
                        if let Some(p) = parse_selection_parameter(&param_rec) {
                            parameters.push(p);
                        }
                    }
                }
            }
            Some("SelectOptions") => {
                for coll in children_by_tag(&pv, "Collection") {
                    for opt_rec in children_by_tag(&coll, "Record") {
                        if let Some(o) = parse_select_option(&opt_rec) {
                            select_options.push(o);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Some(SelectionVariant {
        qualifier: None,
        text,
        parameters,
        select_options,
    })
}

/// Parse a `<Record Type="UI.ParameterType">` — a single-valued filter
/// constraint with a literal `PropertyValue`.
fn parse_selection_parameter(record: &roxmltree::Node) -> Option<SelectionParameter> {
    let mut property_name: Option<String> = None;
    let mut property_value: Option<String> = None;
    for pv in children_by_tag(record, "PropertyValue") {
        match pv.attribute("Property") {
            Some("PropertyName") => {
                property_name = pv.attribute("PropertyPath").map(String::from);
            }
            Some("PropertyValue") => {
                // Parameters take a literal primitive value. SAP emits
                // `String=`, `Int=`, or `Bool=`; fall through so we
                // don't lose numeric/bool defaults.
                property_value = pv
                    .attribute("String")
                    .or_else(|| pv.attribute("Int"))
                    .or_else(|| pv.attribute("Bool"))
                    .map(String::from);
            }
            _ => {}
        }
    }
    Some(SelectionParameter {
        property_name: property_name?,
        property_value: property_value?,
    })
}

/// Parse a `<Record Type="UI.SelectOptionType">` — one property's
/// SELECT-OPTIONS list, with zero or more ranges inside.
fn parse_select_option(record: &roxmltree::Node) -> Option<SelectOption> {
    let mut property_name: Option<String> = None;
    let mut ranges: Vec<SelectionRange> = Vec::new();
    for pv in children_by_tag(record, "PropertyValue") {
        match pv.attribute("Property") {
            Some("PropertyName") => {
                property_name = pv.attribute("PropertyPath").map(String::from);
            }
            Some("Ranges") => {
                for coll in children_by_tag(&pv, "Collection") {
                    for range_rec in children_by_tag(&coll, "Record") {
                        if let Some(r) = parse_selection_range(&range_rec) {
                            ranges.push(r);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let property_name = property_name?;
    if ranges.is_empty() {
        return None;
    }
    Some(SelectOption {
        property_name,
        ranges,
    })
}

/// Parse a `<Record Type="UI.SelectionRangeType">` — one range with
/// sign, operator, and one or two bounds. Unknown enum members short-
/// circuit the whole range (better to drop than to mis-interpret as EQ).
fn parse_selection_range(record: &roxmltree::Node) -> Option<SelectionRange> {
    let mut sign: Option<SelectionSign> = None;
    let mut option: Option<SelectionOption> = None;
    let mut low: Option<String> = None;
    let mut high: Option<String> = None;
    for pv in children_by_tag(record, "PropertyValue") {
        match pv.attribute("Property") {
            Some("Sign") => {
                if let Some(em) = pv.attribute("EnumMember") {
                    let leaf = em.rsplit('/').next().unwrap_or(em);
                    sign = match leaf {
                        "I" => Some(SelectionSign::I),
                        "E" => Some(SelectionSign::E),
                        _ => None,
                    };
                }
            }
            Some("Option") => {
                if let Some(em) = pv.attribute("EnumMember") {
                    let leaf = em.rsplit('/').next().unwrap_or(em);
                    option = match leaf {
                        "EQ" => Some(SelectionOption::Eq),
                        "NE" => Some(SelectionOption::Ne),
                        "GT" => Some(SelectionOption::Gt),
                        "GE" => Some(SelectionOption::Ge),
                        "LT" => Some(SelectionOption::Lt),
                        "LE" => Some(SelectionOption::Le),
                        "BT" => Some(SelectionOption::Bt),
                        "NB" => Some(SelectionOption::Nb),
                        "CP" => Some(SelectionOption::Cp),
                        "NP" => Some(SelectionOption::Np),
                        _ => None,
                    };
                }
            }
            Some("Low") => {
                low = pv
                    .attribute("String")
                    .or_else(|| pv.attribute("Int"))
                    .or_else(|| pv.attribute("Bool"))
                    .map(String::from);
            }
            Some("High") => {
                high = pv
                    .attribute("String")
                    .or_else(|| pv.attribute("Int"))
                    .or_else(|| pv.attribute("Bool"))
                    .map(String::from);
            }
            _ => {}
        }
    }
    Some(SelectionRange {
        sign: sign?,
        option: option?,
        low: low?,
        high,
    })
}

/// Pull the interesting fields out of a `UI.PresentationVariant`
/// record: `RequestAtLeast` (property paths added to `$select`) and
/// `SortOrder` (default `$orderby` clauses). Empty tuple when the
/// record is missing or both collections are unset.
fn parse_presentation_variant(annot: &roxmltree::Node) -> (Vec<String>, Vec<SortOrder>) {
    let mut request_at_least: Vec<String> = Vec::new();
    let mut sort_order: Vec<SortOrder> = Vec::new();
    let record = match children_by_tag(annot, "Record").into_iter().next() {
        Some(r) => r,
        None => return (request_at_least, sort_order),
    };
    for pv in children_by_tag(&record, "PropertyValue") {
        match pv.attribute("Property") {
            Some("RequestAtLeast") => {
                for coll in children_by_tag(&pv, "Collection") {
                    for pp in children_by_tag(&coll, "PropertyPath") {
                        if let Some(text) = pp.text() {
                            let trimmed = text.trim();
                            if !trimmed.is_empty()
                                && !request_at_least.iter().any(|x: &String| x == trimmed)
                            {
                                request_at_least.push(trimmed.to_string());
                            }
                        }
                    }
                }
            }
            Some("SortOrder") => {
                for coll in children_by_tag(&pv, "Collection") {
                    for sort_rec in children_by_tag(&coll, "Record") {
                        if let Some(so) = parse_sort_order_record(&sort_rec) {
                            sort_order.push(so);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    (request_at_least, sort_order)
}

/// Parse one `<Record Type="Common.SortOrderType">` (or untyped) from
/// the `SortOrder` collection. Shape: `Property` (PropertyPath) +
/// optional `Descending` (Bool, default false).
fn parse_sort_order_record(record: &roxmltree::Node) -> Option<SortOrder> {
    let mut property: Option<String> = None;
    let mut descending = false;
    for pv in children_by_tag(record, "PropertyValue") {
        match pv.attribute("Property") {
            Some("Property") => {
                property = pv.attribute("PropertyPath").map(String::from);
            }
            Some("Descending") => {
                descending = pv.attribute("Bool").map(|v| v == "true").unwrap_or(false);
            }
            _ => {}
        }
    }
    Some(SortOrder {
        property: property?,
        descending,
    })
}

/// Walk a `UI.LineItem` annotation's `<Collection>` of `<Record>`s and
/// return one `LineItemField` per `UI.DataField` record that has a
/// `Value Path="..."`. Records whose `Type` is `UI.DataFieldFor*`
/// (Action, Annotation, IntentBasedNavigation, ...) are skipped — they
/// don't map to `$select`-able columns.
fn parse_line_item_collection(annot: &roxmltree::Node) -> Vec<LineItemField> {
    let mut out = Vec::new();
    let collection = match children_by_tag(annot, "Collection").into_iter().next() {
        Some(c) => c,
        None => return out,
    };
    for record in children_by_tag(&collection, "Record") {
        // Record Type defaults to "UI.DataField" when omitted. Accept
        // the bare name, the SAP__ alias, and the fully-qualified form.
        let type_attr = record.attribute("Type").unwrap_or("UI.DataField");
        let normalized = type_attr.trim_start_matches("SAP__");
        let leaf = normalized.rsplit('.').next().unwrap_or(normalized);
        if leaf != "DataField" {
            continue;
        }
        let mut value_path: Option<String> = None;
        let mut label: Option<String> = None;
        let mut importance: Option<String> = None;
        for pv in children_by_tag(&record, "PropertyValue") {
            match pv.attribute("Property") {
                Some("Value") => value_path = pv.attribute("Path").map(String::from),
                Some("Label") => label = pv.attribute("String").map(String::from),
                Some("Importance") => {
                    if let Some(em) = pv.attribute("EnumMember") {
                        importance = Some(em.rsplit('/').next().unwrap_or(em).to_string());
                    }
                }
                _ => {}
            }
        }
        if let Some(vp) = value_path {
            out.push(LineItemField {
                value_path: vp,
                label,
                importance,
            });
        }
    }
    out
}

/// Read a `<PropertyValue>` whose content is a `<Collection>` of
/// `<PropertyPath>` text nodes. Returns the path strings in source order.
fn parse_property_path_collection(pv: &roxmltree::Node) -> Vec<String> {
    let mut out = Vec::new();
    for coll in children_by_tag(pv, "Collection") {
        for pp in children_by_tag(&coll, "PropertyPath") {
            if let Some(text) = pp.text() {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    out.push(trimmed.to_string());
                }
            }
        }
    }
    out
}

/// Extract a `Common.FieldControl` value from an `<Annotation>` node.
/// Accepts `EnumMember` for the five named states, `Path=` for dynamic
/// (per-row) control, and the legacy `Int=` form that SAP-generated
/// services still sometimes emit (7=Mandatory, 3=Optional, 1=ReadOnly,
/// 0=Inapplicable, 5=Hidden).
fn parse_field_control(annot: &roxmltree::Node) -> Option<FieldControl> {
    if let Some(path) = annot.attribute("Path") {
        return Some(FieldControl::Path(path.to_string()));
    }
    if let Some(em) = annot.attribute("EnumMember") {
        let leaf = em.rsplit('/').next().unwrap_or(em);
        return match leaf {
            "Mandatory" => Some(FieldControl::Mandatory),
            "Optional" => Some(FieldControl::Optional),
            "ReadOnly" => Some(FieldControl::ReadOnly),
            "Inapplicable" => Some(FieldControl::Inapplicable),
            "Hidden" => Some(FieldControl::Hidden),
            _ => None,
        };
    }
    if let Some(int_str) = annot.attribute("Int")
        && let Ok(n) = int_str.parse::<u8>()
    {
        return match n {
            7 => Some(FieldControl::Mandatory),
            3 => Some(FieldControl::Optional),
            1 => Some(FieldControl::ReadOnly),
            0 => Some(FieldControl::Inapplicable),
            5 => Some(FieldControl::Hidden),
            _ => None,
        };
    }
    None
}

/// Extract a `UI.TextArrangement` value from an `<Annotation>` node.
/// Accepts the `EnumMember="UI.TextArrangementType/TextFirst"` form
/// (with or without the `SAP__` namespace alias) and the SAP-shorthand
/// `TextFirst` unqualified. Unknown variants return `None` rather than
/// guessing — we'd rather fall back to Fiori's default than mis-render.
fn parse_text_arrangement(annot: &roxmltree::Node) -> Option<TextArrangement> {
    let em = annot.attribute("EnumMember")?;
    let leaf = em.rsplit('/').next().unwrap_or(em);
    match leaf {
        "TextFirst" => Some(TextArrangement::TextFirst),
        "TextLast" => Some(TextArrangement::TextLast),
        "TextSeparate" => Some(TextArrangement::TextSeparate),
        "TextOnly" => Some(TextArrangement::TextOnly),
        _ => None,
    }
}

/// Extract a `UI.Criticality` value from a `<Annotation>` node. Accepts
/// the three common inline forms: `Path="..."`, `Int="n"`, or
/// `EnumMember="UI.CriticalityType/Positive"`. Anything else (nested
/// records, expressions) is ignored for now.
fn parse_criticality(annot: &roxmltree::Node) -> Option<Criticality> {
    if let Some(path) = annot.attribute("Path") {
        return Some(Criticality::Path(path.to_string()));
    }
    if let Some(int_str) = annot.attribute("Int")
        && let Ok(n) = int_str.parse::<u8>()
    {
        return Some(Criticality::Fixed(n));
    }
    if let Some(em) = annot.attribute("EnumMember") {
        // "UI.CriticalityType/Positive" (or SAP-aliased). Take the last segment.
        let name = em.rsplit('/').next().unwrap_or(em);
        let level = match name {
            "Neutral" | "VeryNegative" => 0, // SAP "VeryNegative" is a legacy alias; treat as 0 to avoid misclassification.
            "Negative" => 1,
            "Critical" => 2,
            "Positive" | "VeryPositive" => 3,
            "Information" => 5,
            _ => return None,
        };
        return Some(Criticality::Fixed(level));
    }
    None
}

/// Parse the `Record` inside a `UI.HeaderInfo` annotation. Returns `None`
/// if the annotation is missing its Record child or has no useful fields.
fn parse_header_info_record(annot: &roxmltree::Node) -> Option<HeaderInfo> {
    let record = children_by_tag(annot, "Record").into_iter().next()?;
    let mut info = HeaderInfo::default();
    for pv in children_by_tag(&record, "PropertyValue") {
        match pv.attribute("Property") {
            Some("TypeName") => info.type_name = pv.attribute("String").map(String::from),
            Some("TypeNamePlural") => {
                info.type_name_plural = pv.attribute("String").map(String::from);
            }
            Some("Title") => {
                // Title is itself a Record { Value: Path } (UI.DataField shape).
                if let Some(title_rec) = children_by_tag(&pv, "Record").into_iter().next() {
                    for title_pv in children_by_tag(&title_rec, "PropertyValue") {
                        if title_pv.attribute("Property") == Some("Value") {
                            info.title_path = title_pv.attribute("Path").map(String::from);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if info.type_name.is_none() && info.type_name_plural.is_none() && info.title_path.is_none() {
        None
    } else {
        Some(info)
    }
}

/// Scan an F4 service's `$metadata` XML for a `Common.ValueListMapping`
/// annotation that targets `local_property` (matched on the trailing
/// path segment — the target's namespace-qualified entity type doesn't
/// have to match). Returns the first match as a `ValueList` — the same
/// record shape our inline-`Common.ValueList` parser already handles.
///
/// This is the bridge between the parent service (which only carries
/// `Common.ValueListReferences` URLs) and the F4 service (which carries
/// the actual `Common.ValueListMapping` record). Callers resolve the
/// reference URL, fetch the F4 `$metadata`, then hand it to this helper
/// along with the parent property name.
pub fn parse_value_list_mapping_xml(xml: &str, local_property: &str) -> Option<ValueList> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let schema_nodes: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name("Schema"))
        .collect();
    let suffix = format!("/{local_property}");
    for schema in schema_nodes {
        for annotations_block in children_by_tag(&schema, "Annotations") {
            let target = match annotations_block.attribute("Target") {
                Some(t) => t,
                None => continue,
            };
            if !target.ends_with(&suffix) {
                continue;
            }
            for annot in children_by_tag(&annotations_block, "Annotation") {
                let term = match annot.attribute("Term") {
                    Some(t) => t,
                    None => continue,
                };
                let lower = term.trim_start_matches("SAP__").to_ascii_lowercase();
                if !lower.ends_with(".valuelistmapping") {
                    continue;
                }
                if let Some(vl) = parse_value_list_record(&annot) {
                    return Some(vl);
                }
            }
        }
    }
    None
}
