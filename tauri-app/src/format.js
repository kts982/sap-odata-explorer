// ── Display formatting helpers ──
// Pure functions that turn raw EDM values + SAP annotation hints into
// presentation-layer strings. The HTML-emitting helpers build their
// snippets with safeHtml so callers can raw() the returned fragments.

import { safeHtml } from './html.js';

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
  return safeHtml`<span class="${color} mr-1" title="UI.Criticality (${src}) = ${label}">&#9679;</span>`;
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

// OData version ('V2' | 'V4') of a tab's service: the $metadata-derived
// value once loaded, else SAP's path convention (/sap/opu/odata4/ = V4).
export function serviceODataVersion(tab) {
  if (tab && tab.serviceVersion) return tab.serviceVersion;
  const path = (tab && tab.servicePath) || '';
  return path.includes('/sap/opu/odata/') ? 'V2' : 'V4';
}

// `/Date(1700000000000)/` or `/Date(1700000000000+0060)/` — the V2 JSON
// date shape, which is what result cells and value-help rows carry.
function parseMsDate(s) {
  const m = /^\/Date\((-?\d+)(?:[+-]\d{4})?\)\/$/.exec(s);
  return m ? new Date(Number(m[1])) : null;
}

function isoWithoutMillis(d) {
  return d.toISOString().replace(/\.000Z$/, 'Z');
}

// `13:20` / `13:20:05` → `PT13H20M05S` (V2 Edm.Time).
function clockToDuration(s) {
  const m = /^(\d{1,2}):(\d{2})(?::(\d{2}))?$/.exec(s);
  return m ? `PT${m[1].padStart(2, '0')}H${m[2]}M${m[3] || '00'}S` : s;
}

// `PT13H20M05S` → `13:20:05` (V4 Edm.TimeOfDay).
function durationToClock(s) {
  const m = /^PT(?:(\d+)H)?(?:(\d+)M)?(?:(\d+(?:\.\d+)?)S)?$/.exec(s);
  if (!m) return s;
  const pad = v => String(v || 0).padStart(2, '0');
  return `${pad(m[1])}:${pad(m[2])}:${pad(m[3])}`;
}

const NUMERIC_LITERAL = /^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/;

// OData literal for `<property> eq <lit>`-style clauses, by EDM type and
// OData version. V2 and V4 disagree on most non-string types: V2 needs
// `guid'…'`, `datetime'…'`, `datetimeoffset'…'`, `time'PT…'`; V4 writes
// GUIDs, dates, times and timestamps bare. Values are accepted in the
// shapes the UI sees — what the user typed, or a raw V2/V4 JSON value
// from a result cell / value-help row (`/Date(…)/`, `PT…`, ISO).
// Unknown types fall back to a quoted string.
export function formatODataLiteral(value, edmType, version = 'V4') {
  if (value === null || value === undefined) return 'null';
  const t = (edmType || '').replace(/^Edm\./, '');
  const v2 = version === 'V2';
  const s = String(value);
  const quoted = () => `'${s.replace(/'/g, "''")}'`;
  const bare = s.trim();
  switch (t) {
    case 'Boolean':
      return bare === 'true' || bare === 'false' ? bare : quoted();
    case 'Byte':
    case 'SByte':
    case 'Int16':
    case 'Int32':
    case 'Int64':
    case 'Decimal':
    case 'Double':
    case 'Single':
      // Anything non-numeric stays quoted so the server reports a type
      // error instead of reading it as part of the expression.
      return NUMERIC_LITERAL.test(bare) ? bare : quoted();
    case 'Guid': {
      const g = bare.replace(/^guid'(.*)'$/i, '$1');
      return v2 ? `guid'${g}'` : g;
    }
    case 'DateTime': {
      // V2-only type, offset-less: datetime'2026-01-01T00:00:00'.
      const d = parseMsDate(bare);
      let iso = d ? isoWithoutMillis(d).replace(/Z$/, '') : bare;
      if (/^\d{4}-\d{2}-\d{2}$/.test(iso)) iso += 'T00:00:00';
      return `datetime'${iso}'`;
    }
    case 'DateTimeOffset': {
      const d = parseMsDate(bare);
      let iso = d ? isoWithoutMillis(d) : bare;
      if (/^\d{4}-\d{2}-\d{2}$/.test(iso)) iso += 'T00:00:00Z';
      return v2 ? `datetimeoffset'${iso}'` : iso;
    }
    case 'Date': {
      const d = parseMsDate(bare);
      return d ? d.toISOString().slice(0, 10) : bare;
    }
    case 'TimeOfDay':
      return durationToClock(bare);
    case 'Time':
      return `time'${clockToDuration(bare)}'`;
    case 'Duration':
      return `duration'${bare}'`;
    default:
      return quoted();
  }
}

// Quote one argument so a command pastes into Bash and PowerShell alike:
// bare when it's plain, double quotes in the common case (OData filters
// are full of single quotes), single quotes when the value holds a
// character either shell expands inside double quotes.
export function shellArg(value) {
  const s = String(value);
  if (/^[A-Za-z0-9_./:,=@+-]+$/.test(s)) return s;
  if (!/["$`\\]/.test(s)) return `"${s}"`;
  return `'${s.replace(/'/g, `'\\''`)}'`;
}

// The `sap-odata … run …` command equivalent to a desktop query. `-s` is a
// connection flag and must precede the subcommand.
export function buildCliCommand(profile, servicePath, params) {
  const parts = ['sap-odata', '-p', shellArg(profile), '-s', shellArg(servicePath), 'run', shellArg(params.entity_set)];
  const opt = (flag, v) => {
    if (v !== null && v !== undefined && v !== '') parts.push(flag, shellArg(v));
  };
  opt('--select', params.select);
  opt('--filter', params.filter);
  opt('--expand', params.expand);
  opt('--orderby', params.orderby);
  opt('--top', params.top);
  opt('--skip', params.skip);
  return parts.join(' ');
}

// Total row count from an inline-count response, or null.
// V4 `@odata.count`; V2 `d.__count` (a string).
export function extractTotalCount(data) {
  if (!data || typeof data !== 'object') return null;
  const raw = data['@odata.count'] ?? (data.d && data.d.__count);
  const n = Number(raw);
  return raw === undefined || raw === null || Number.isNaN(n) ? null : n;
}

// `$skiptoken` of a server-driven next link (V4 `@odata.nextLink`, V2
// `d.__next`), or null when the server didn't page.
export function nextSkiptoken(data) {
  if (!data || typeof data !== 'object') return null;
  const link = data['@odata.nextLink'] ?? (data.d && data.d.__next);
  if (typeof link !== 'string') return null;
  const m = /[?&]\$skiptoken=([^&]*)/.exec(link);
  return m ? decodeURIComponent(m[1]) : null;
}

// Paging state for the stats bar, from the response and the params that
// produced it. `offset` is known from $skip, or from a numeric
// $skiptoken (SAP's RAP tokens are row offsets).
export function pagingInfo(data, params, rowCount) {
  const total = extractTotalCount(data);
  const token = nextSkiptoken(data);
  const pageSize = params.top || rowCount;
  const tokenOffset = params.skiptoken && /^\d+$/.test(params.skiptoken) ? Number(params.skiptoken) : null;
  const offset = params.skip || tokenOffset || 0;
  const hasMore = token !== null
    || (total !== null ? offset + rowCount < total : !!params.top && rowCount === params.top);
  return {
    offset,
    rowCount,
    total,
    nextSkiptoken: token,
    nextSkip: token === null && hasMore ? offset + pageSize : null,
    prevSkip: offset > 0 && (params.skip || tokenOffset !== null) ? Math.max(0, offset - pageSize) : null,
    hasMore,
  };
}

// Rows as delimited text (TSV for pasting into Excel, CSV). Columns are
// the first row's keys minus OData bookkeeping (`@…`, `__metadata`);
// nested values become compact JSON, null becomes empty. Fields holding
// the delimiter, a quote or a line break are quoted, quotes doubled.
export function toDelimited(rows, delimiter) {
  if (!Array.isArray(rows) || rows.length === 0) return '';
  const cols = Object.keys(rows[0]).filter(k => !k.startsWith('@') && k !== '__metadata');
  const field = v => {
    if (v === null || v === undefined) return '';
    const s = typeof v === 'object' ? JSON.stringify(v) : String(v);
    return s.includes(delimiter) || /["\r\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
  };
  const lines = [cols.map(field).join(delimiter)];
  for (const row of rows) lines.push(cols.map(c => field(row[c])).join(delimiter));
  return lines.join('\r\n');
}

// `contains(prop, lit)` in V4; V2 has no `contains` and spells the same
// test `substringof(lit, prop)`.
export function containsClause(property, literal, version) {
  return version === 'V2'
    ? `substringof(${literal},${property})`
    : `contains(${property},${literal})`;
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
  return safeHtml` <button type="button" class="text-[9px] font-semibold tracking-wide px-1 py-px rounded-sm ${cls} transition-colors align-middle" data-action="value-list" data-prop="${p.name}" data-kind="${kind}" title="${tip}">&#x21D2; F4</button>`;
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
    return safeHtml` <span class="${color} text-[10px]" title="UI.Criticality = ${label}">&#9679;</span>`;
  }
  if (c.kind === 'path') {
    return safeHtml` <span class="text-ox-blue text-[10px]" title="UI.Criticality Path = ${c.value}">&#8680; ${c.value}</span>`;
  }
  return '';
}
