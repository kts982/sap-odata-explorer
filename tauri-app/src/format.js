// ── Display formatting helpers ──
// Pure functions that turn raw EDM values + SAP annotation hints into
// presentation-layer strings. The HTML-emitting helpers escape every
// interpolation via escapeHtml; callers assemble the returned strings
// into safeHtml templates before innerHTML assignment.

import { escapeHtml } from './html.js';

// Render a small colored dot for a cell's UI.Criticality when SAP
// View is on. Fixed criticality paints the same level for every row;
// Path criticality reads the numeric code from a sibling column per
// row. Codes follow the OData spec: 0=Neutral, 1=Negative, 2=Critical,
// 3=Positive, 5=Information. Unknown levels render as neutral dim.
// Returns an empty string when there's nothing to show (including
// when the Path column is missing or the level is 0/Neutral — 0 is
// the default-good state and doesn't need visual marking).
export function criticalityDot(prop, row) {
  if (!prop || !prop.criticality) return '';
  const c = prop.criticality;
  let level;
  if (c.kind === 'fixed') {
    level = c.value;
  } else if (c.kind === 'path') {
    const raw = row[c.value];
    if (raw === null || raw === undefined || raw === '') return '';
    const n = parseInt(String(raw), 10);
    if (isNaN(n)) return '';
    level = n;
  } else {
    return '';
  }
  if (level === 0) return '';
  const color = level === 3 ? 'text-ox-green' :
                level === 2 ? 'text-ox-amber' :
                level === 1 ? 'text-ox-red' :
                level === 5 ? 'text-ox-blue' : '';
  if (!color) return '';
  const label = level === 3 ? 'positive' :
                level === 2 ? 'critical' :
                level === 1 ? 'negative' :
                level === 5 ? 'info' : '';
  const src = c.kind === 'path' ? `via ${c.value}` : 'fixed';
  return `<span class="${color} mr-1" title="UI.Criticality (${src}) = ${label}">&#9679;</span>`;
}

// Apply a V2 `sap:display-format` hint to a raw cell value for
// results-grid rendering. Leaves `$filter`/click-to-filter values
// untouched — the caller keeps the raw string in data attributes.
// Common hints SAP services emit:
//   "Date"        — drop the time portion; handles both V4 ISO 8601
//                   (YYYY-MM-DDTHH:MM:SS) and V2's `/Date(ms)/`.
//   "Time"        — keep just HH:MM:SS.
//   "UpperCase"   — uppercase the whole string.
//   "NonNegative" — coerce negatives to 0 (spec defines the field as
//                   non-negative; negative should never appear, but
//                   surface it as 0 rather than a wrong-looking sign).
// Anything else falls through unchanged.
export function formatDisplayValue(raw, displayFormat, edmType) {
  if (!displayFormat || raw === '' || raw === null || raw === undefined) return raw;
  const fmt = String(displayFormat).toLowerCase();
  const s = String(raw);
  switch (fmt) {
    case 'date':
      return formatSapDate(s);
    case 'time':
      return formatSapTime(s);
    case 'uppercase':
      return s.toUpperCase();
    case 'nonnegative': {
      const n = Number(s);
      if (!isNaN(n) && n < 0) return '0';
      return s;
    }
    default:
      void edmType;
      return s;
  }
}

// Normalize a date-ish SAP value to `YYYY-MM-DD`. Handles:
//   - V4 ISO timestamps: `2026-04-21T10:15:00Z` → `2026-04-21`
//   - V2 `/Date(1234567890000)/` (with optional timezone suffix) →
//     `YYYY-MM-DD` via Date parse
//   - V2 `Edm.DateTime` already in `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM:SS`
// Falls back to the raw string if none of those match.
export function formatSapDate(s) {
  const m = s.match(/^\/Date\((-?\d+)(?:[+-]\d+)?\)\/$/);
  if (m) {
    const ts = parseInt(m[1], 10);
    if (!isNaN(ts)) return new Date(ts).toISOString().slice(0, 10);
  }
  const iso = s.match(/^(\d{4}-\d{2}-\d{2})(?:T|$)/);
  if (iso) return iso[1];
  return s;
}

export function formatSapTime(s) {
  // V4 duration-like `PT10H15M` → rebuild as HH:MM:SS (common in V2).
  const dur = s.match(/^PT(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?$/);
  if (dur) {
    const h = String(parseInt(dur[1] || '0', 10)).padStart(2, '0');
    const m = String(parseInt(dur[2] || '0', 10)).padStart(2, '0');
    const sec = String(parseInt(dur[3] || '0', 10)).padStart(2, '0');
    return `${h}:${m}:${sec}`;
  }
  const time = s.match(/^(\d{2}:\d{2}(?::\d{2})?)/);
  if (time) return time[1];
  return s;
}

// OData v4 literal formatting by edm type — enough for the picker's
// `local eq <lit>` clauses. Falls back to single-quoted strings for
// unknown types since that's what SAP services overwhelmingly expect.
export function formatODataLiteral(value, edmType) {
  const t = (edmType || '').replace('Edm.', '');
  const s = String(value);
  switch (t) {
    case 'Boolean':
      return s === 'true' || s === 'false' ? s : `'${s.replace(/'/g, "''")}'`;
    case 'Byte':
    case 'SByte':
    case 'Int16':
    case 'Int32':
    case 'Int64':
    case 'Decimal':
    case 'Double':
    case 'Single':
      return s;
    case 'Guid':
      return `guid'${s}'`;
    case 'DateTime':
      return `datetime'${s}'`;
    case 'DateTimeOffset':
      return s;
    default:
      return `'${s.replace(/'/g, "''")}'`;
  }
}

// Compact summary of a ValueList's parameter bindings for the marker
// tooltip, e.g. "Warehouse↔Warehouse, Plant→Plant, Language=EN, (Desc)".
// Kept short on purpose — the picker modal shows the full mapping.
export function valueListSummary(vl) {
  if (!vl || !Array.isArray(vl.parameters)) return '';
  const bits = vl.parameters.map(p => {
    switch (p.kind) {
      case 'inout':
        return `${p.local_property}↔${p.value_list_property}`;
      case 'in':
        return `${p.local_property}→${p.value_list_property}`;
      case 'out':
        return `${p.value_list_property}→${p.local_property}`;
      case 'constant':
        return `${p.value_list_property}=${p.constant ?? ''}`;
      case 'displayonly':
        return `(${p.value_list_property})`;
      default:
        return p.value_list_property || '?';
    }
  });
  return bits.join(', ');
}

// SAP View marker for properties that have a value help. Covers three
// shapes: inline Common.ValueList, Common.ValueListReferences (URLs to
// separate F4 services), and Common.ValueListWithFixedValues (a marker
// that says "few fixed values"; no mapping to drive, so the picker
// can't offer a real picker — we still show the badge as a hint).
// The button is a self-contained action inside the row — delegated
// handler's `closest('[data-action]')` picks this over the row's
// `select` action so clicking the marker does NOT also add the column
// to `$select`.
//
// `sapViewEnabled` is passed in by the caller (it lives in app.js's
// module-local state until batch 2 lifts it into state.js); the helper
// itself stays pure.
export function valueListHint(p, sapViewEnabled) {
  if (!sapViewEnabled) return '';
  const hasInline = !!p.value_list;
  const refs = Array.isArray(p.value_list_references) ? p.value_list_references : [];
  const hasRefs = refs.length > 0;
  const fixed = p.value_list_fixed === true;
  const v2 = p.sap_value_list;
  if (!hasInline && !hasRefs && !fixed && !v2) return '';
  // Inline > references > fixed > V2 marker — pick class + tooltip
  // accordingly. The lowest-capability variants (fixed, V2) get the
  // mutest style since they're hints without a picker target.
  let cls;
  let tip;
  let kind;
  if (hasInline) {
    const vl = p.value_list;
    const label = vl.label ? `${vl.label}\n` : '';
    tip = `${label}Value help → ${vl.collection_path}${vl.search_supported === true ? ' ($search)' : ''}\n${valueListSummary(vl)}`;
    cls = 'text-ox-electric border border-ox-electric/50 hover:bg-ox-electric/10 hover:border-ox-electric';
    kind = 'inline';
  } else if (hasRefs) {
    tip = `Referenced value help (${refs.length} ref${refs.length > 1 ? 's' : ''}) — resolved on open:\n${refs.join('\n')}`;
    // Dashed border signals "external reference, resolution required".
    cls = 'text-ox-electric border border-dashed border-ox-electric/60 hover:bg-ox-electric/10 hover:border-ox-electric';
    kind = 'refs';
  } else if (fixed) {
    // Fixed-values only — no mapping, just a Fiori "dropdown-worthy" hint.
    tip = 'Common.ValueListWithFixedValues — property has a fixed value set but no ValueList mapping in this service.';
    cls = 'text-ox-dim border border-ox-dim/50 cursor-help';
    kind = 'fixed';
  } else {
    // V2 sap:value-list marker — no mapping record in the metadata, so
    // no picker target. Still worth surfacing because Fiori lights up
    // a value help on this property at runtime via naming convention
    // or a sibling nav prop; the user just can't drive it from here.
    const flavour = v2 === 'fixed-values' ? 'fixed-values' : 'standard';
    tip = `sap:value-list="${flavour}" — V2 service declares a value help, but V2 metadata doesn't carry the mapping. Fiori resolves it by convention at runtime; no picker available here.`;
    cls = 'text-ox-dim border border-ox-dim/50 cursor-help';
    kind = 'v2';
  }
  return ` <button type="button" class="text-[9px] font-semibold tracking-wide px-1 py-px rounded-sm ${cls} transition-colors align-middle" data-action="value-list" data-prop="${escapeHtml(p.name)}" data-kind="${kind}" title="${escapeHtml(tip)}">&#x21D2; F4</button>`;
}

// SAP View hint for UI.Criticality declared on a property: a small
// dot for fixed criticality, an arrow + path label for path-based
// (0 Neutral, 1 Negative, 2 Critical, 3 Positive, 5 Information). Path
// criticality renders as "⇢ TargetProp" so the user can see where the
// value comes from at runtime.
export function criticalityHint(p) {
  const c = p.criticality;
  if (!c) return '';
  if (c.kind === 'fixed') {
    const level = c.value;
    const color = level === 3 ? 'text-ox-green' :
                  level === 2 ? 'text-ox-amber' :
                  level === 1 ? 'text-ox-red' :
                  level === 5 ? 'text-ox-blue' : 'text-ox-dim';
    const label = level === 3 ? 'positive' :
                  level === 2 ? 'critical' :
                  level === 1 ? 'negative' :
                  level === 5 ? 'info' : 'neutral';
    return ` <span class="${color} text-[10px]" title="UI.Criticality = ${label}">&#9679;</span>`;
  }
  if (c.kind === 'path') {
    return ` <span class="text-ox-blue text-[10px]" title="UI.Criticality Path = ${escapeHtml(c.value)}">&#8680; ${escapeHtml(c.value)}</span>`;
  }
  return '';
}
