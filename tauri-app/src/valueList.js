// ── Value-list (F4) picker ──
//
// Stateful + async. The picker modal opens for a property that has
// either an inline Common.ValueList, one or more
// Common.ValueListReferences, or both — the variant pill bar lets the
// user switch between them. Reference variants resolve lazily through
// the Tauri `resolve_value_list_reference` command and the result is
// memoised in `_vlResolveCache` (per absolute URL) so reopens stay
// instant.
//
// Module-level mutables (`_vl*`) are intentionally not in state.js —
// they're picker-private and have no readers outside this module.
//
// All imports flow downward — no circular back to app.js.

import { state } from './state.js';
import { safeHtml, escapeHtml } from './html.js';
import { valueListSummary, formatODataLiteral } from './format.js';
import { setStatus } from './status.js';
import { timedInvoke } from './api.js';
import { getActiveTab } from './tabs.js';
import { extractRows } from './results.js';

// State held per-open: the full Property info, the ValueList that will
// drive the picker (inline for same-service value help, resolved for
// reference-backed), and the service path to fetch rows from (the
// current service's for inline, the F4 service's for references).
let _vlActiveProperty = null;
let _vlActiveValueList = null;
let _vlActiveServicePath = null;
// Cache resolved references across opens so reopening the same F4 is
// instant. Key: absolute reference URL (NOT property name — the same
// F4 can back many properties).
const _vlResolveCache = new Map();

// Known variants for the active property, in the order they appear in
// the modal's pill bar. Two kinds:
//   { kind: 'inline', valueList, servicePath: state.currentServicePath, label }
//   { kind: 'reference', url, label, resolved?: { value_list, resolved_service_path } }
// Reference variants are resolved lazily the first time they're
// selected; the result is memoized both here AND in the long-lived
// _vlResolveCache (keyed by URL) so reopens stay instant.
let _vlVariants = [];
let _vlActiveVariantIndex = 0;

export async function openValueListPicker(propertyName) {
  const tab = getActiveTab();
  const info = tab && tab._lastDescribeInfo;
  if (!info) return;
  const prop = info.properties.find(p => p.name === propertyName);
  if (!prop) return;
  const inlineVariants = Array.isArray(prop.value_list_variants) ? prop.value_list_variants : [];
  const refs = Array.isArray(prop.value_list_references) ? prop.value_list_references : [];
  const fixedOnly = inlineVariants.length === 0 && refs.length === 0 && prop.value_list_fixed === true;
  if (fixedOnly) {
    setStatus('This property has fixed values but no ValueList mapping in this service.');
    return;
  }
  if (inlineVariants.length === 0 && refs.length === 0) {
    // V2 sap:value-list marker without a V4 mapping — surface the hint
    // rather than open an empty modal.
    if (prop.sap_value_list) {
      const flavour = prop.sap_value_list === 'fixed-values' ? 'fixed-values' : 'standard';
      setStatus(`V2 sap:value-list="${flavour}" — no mapping record in $metadata. Open the service in Fiori for a runtime-resolved picker.`);
    }
    return;
  }
  _vlActiveProperty = prop;
  _vlVariants = [];
  for (const vl of inlineVariants) {
    _vlVariants.push({
      kind: 'inline',
      valueList: vl,
      servicePath: state.currentServicePath,
      label: vl.qualifier || vl.label || vl.collection_path || 'default',
    });
  }
  for (const url of refs) {
    // Label = the human-sensible chunk of the reference URL (the F4
    // service name), e.g. "c_travelexpensebpgeneralvh".
    const label = vlReferenceLabel(url);
    const cached = _vlResolveCache.get(url);
    _vlVariants.push({
      kind: 'reference',
      url,
      label,
      resolved: cached || null,
    });
  }
  // Clear volatile inputs on fresh open.
  const search = document.getElementById('vlSearch');
  if (search) { search.value = ''; search.classList.add('hidden'); }
  document.getElementById('valueListModal').classList.remove('hidden');
  document.getElementById('vlTitle').textContent = `Value Help · ${prop.name}`;
  document.getElementById('vlSubtitle').textContent = '';
  document.getElementById('vlMapping').textContent = '';
  document.getElementById('vlResults').innerHTML = '<div class="p-4 text-ox-dim text-[11px]">Loading…</div>';
  document.getElementById('vlStatus').textContent = 'Ready.';
  // Activate the first usable variant. selectVariant handles both
  // inline (instant) and reference (may need resolving) shapes.
  await selectVariant(0);
}

// Extract a short label from a ValueListReferences URL — the segment
// just before the matrix-params `;ps=...`, which is the F4 service's
// technical name (e.g. `c_travelexpensebpgeneralvh`).
function vlReferenceLabel(url) {
  try {
    const head = url.split(';')[0];
    const segs = head.split('/').filter(Boolean);
    return segs[segs.length - 2] || segs[segs.length - 1] || url;
  } catch {
    return url;
  }
}

// Switch the active variant. For inline variants this is synchronous.
// For reference variants, kicks off resolve_value_list_reference if
// we haven't already (cache first). Re-renders the whole picker body
// so the mapping, placeholder, and search-visibility match the new
// active ValueList.
export async function selectVariant(index) {
  if (!_vlVariants[index]) return;
  _vlActiveVariantIndex = index;
  const variant = _vlVariants[index];
  const tab = getActiveTab();
  const info = tab && tab._lastDescribeInfo;
  const prop = _vlActiveProperty;
  const title = document.getElementById('vlTitle');
  const subtitle = document.getElementById('vlSubtitle');
  const mapping = document.getElementById('vlMapping');
  const filter = document.getElementById('vlFilter');
  const results = document.getElementById('vlResults');
  const status = document.getElementById('vlStatus');
  renderVlVariantBar();
  if (variant.kind === 'inline') {
    _vlActiveValueList = variant.valueList;
    _vlActiveServicePath = variant.servicePath;
    title.textContent = variant.valueList.label || `Value Help · ${prop.name}`;
    subtitle.textContent = variant.valueList.collection_path;
    mapping.textContent = `Mapping: ${valueListSummary(variant.valueList)}`;
    filter.value = buildInitialVlFilter(prop, info, variant.valueList);
    updateVlSearchVisibility();
    updateVlFilterPlaceholder();
    results.innerHTML = '<div class="p-4 text-ox-dim text-[11px]">Click Fetch to load values.</div>';
    status.textContent = 'Ready.';
    setTimeout(() => filter.focus(), 0);
    return;
  }
  // Reference variant. Resolve (cached if possible) then populate.
  if (!variant.resolved) {
    subtitle.textContent = 'resolving reference…';
    mapping.textContent = '';
    filter.value = '';
    results.innerHTML = '<div class="p-4 text-ox-dim text-[11px]">Resolving value-help service…</div>';
    status.textContent = 'Resolving…';
    try {
      const resolved = await timedInvoke('resolve_value_list_reference', {
        profileName: state.currentProfile,
        servicePath: state.currentServicePath,
        referenceUrl: variant.url,
        localProperty: prop.name,
      });
      _vlResolveCache.set(variant.url, resolved);
      variant.resolved = resolved;
    } catch (e) {
      status.textContent = 'Resolve error';
      results.innerHTML = safeHtml`<div class="p-4 text-ox-red text-[11px]">Could not resolve reference.\n${String(e)}</div>`;
      return;
    }
  }
  _vlActiveValueList = variant.resolved.value_list;
  _vlActiveServicePath = variant.resolved.resolved_service_path;
  title.textContent = variant.resolved.value_list.label || `Value Help · ${prop.name}`;
  subtitle.textContent = variant.resolved.value_list.collection_path;
  mapping.textContent = `Mapping: ${valueListSummary(variant.resolved.value_list)}`;
  filter.value = buildInitialVlFilter(prop, info, variant.resolved.value_list);
  updateVlSearchVisibility();
  updateVlFilterPlaceholder();
  results.innerHTML = '<div class="p-4 text-ox-dim text-[11px]">Click Fetch to load values.</div>';
  status.textContent = `Resolved → ${variant.resolved.resolved_service_path}`;
  setTimeout(() => filter.focus(), 0);
}

// Render the pill bar at the top of the picker so the user can switch
// between value-help sources. Hidden when only one variant exists.
function renderVlVariantBar() {
  const bar = document.getElementById('vlVariantBar');
  const host = document.getElementById('vlVariantPills');
  if (!bar || !host) return;
  if (_vlVariants.length <= 1) {
    bar.classList.add('hidden');
    host.innerHTML = '';
    return;
  }
  bar.classList.remove('hidden');
  host.innerHTML = _vlVariants
    .map((v, i) => {
      const active = i === _vlActiveVariantIndex;
      const base = 'text-[10px] font-semibold tracking-wide px-1.5 py-0.5 rounded-sm transition-colors';
      const onCls = 'text-ox-electric border border-ox-electric bg-ox-electric/10';
      const offCls = 'text-ox-dim border border-ox-border hover:text-ox-electric hover:border-ox-electric/50';
      const cls = active ? `${base} ${onCls}` : `${base} ${offCls}`;
      const kindGlyph = v.kind === 'reference' ? '↗' : '·';
      const title = v.kind === 'reference' ? `Resolved via reference:\n${v.url}` : 'Inline Common.ValueList';
      return `<button type="button" class="${cls}" data-action="vl-select-variant" data-variant-index="${i}" title="${escapeHtml(title)}">${kindGlyph} ${escapeHtml(v.label)}</button>`;
    })
    .join('');
}

// Show the $search input only when the active ValueList declares
// SearchSupported=true. Hidden otherwise so services that don't accept
// $search don't offer a dead control that would 400 on fetch.
function updateVlSearchVisibility() {
  const search = document.getElementById('vlSearch');
  if (!search) return;
  const supported = _vlActiveValueList && _vlActiveValueList.search_supported === true;
  if (supported) {
    search.classList.remove('hidden');
  } else {
    search.classList.add('hidden');
    search.value = '';
  }
}

// Set the picker's $filter placeholder to a workable example using the
// first ValueListProperty from the active mapping (usually the key
// column). Saves the user from guessing that the F4's property name
// is `EWMWarehouse` and not `Warehouse`.
function updateVlFilterPlaceholder() {
  const filter = document.getElementById('vlFilter');
  if (!filter) return;
  const vl = _vlActiveValueList;
  const firstParam = vl && Array.isArray(vl.parameters)
    ? vl.parameters.find(p => p && p.value_list_property)
    : null;
  if (firstParam) {
    filter.placeholder = `$filter (e.g. startswith(${firstParam.value_list_property},'HB'))`;
  } else {
    filter.placeholder = '$filter';
  }
}

export function closeValueListPicker() {
  document.getElementById('valueListModal').classList.add('hidden');
  _vlActiveProperty = null;
  _vlActiveValueList = null;
  _vlActiveServicePath = null;
  _vlVariants = [];
  _vlActiveVariantIndex = 0;
  const bar = document.getElementById('vlVariantBar');
  const host = document.getElementById('vlVariantPills');
  if (bar) bar.classList.add('hidden');
  if (host) host.innerHTML = '';
}

// Pre-seed the VL filter with In/InOut parameter constraints by
// echoing whatever values the main $filter already has on those
// local columns. A lightweight text match — we look for
// `LocalProperty eq <literal>` or `LocalProperty eq '<literal>'`
// patterns. Anything fancier stays empty and the user can type.
function buildInitialVlFilter(prop, info, vl) {
  const mainFilter = (document.getElementById('qFilter').value || '').trim();
  if (!mainFilter) return '';
  const clauses = [];
  for (const param of vl.parameters) {
    if (param.kind !== 'in' && param.kind !== 'inout') continue;
    if (!param.local_property) continue;
    const re = new RegExp(`\\b${param.local_property}\\s+eq\\s+('[^']*'|[-\\w.]+)`);
    const m = mainFilter.match(re);
    if (m) {
      clauses.push(`${param.value_list_property} eq ${m[1]}`);
    }
    if (param.kind === 'inout' && param.local_property === prop.name) {
      // We're opening this picker specifically because the user wants
      // to set `prop`. Don't pre-filter by its own current value.
      clauses.pop();
    }
  }
  // Echo Constant parameters unconditionally — they're fixed filters.
  for (const param of vl.parameters) {
    if (param.kind === 'constant' && param.constant !== null && param.constant !== undefined) {
      const needsQuotes = isNaN(Number(param.constant)) && param.constant !== 'true' && param.constant !== 'false';
      const lit = needsQuotes ? `'${param.constant.replace(/'/g, "''")}'` : param.constant;
      clauses.push(`${param.value_list_property} eq ${lit}`);
    }
  }
  void info;
  return clauses.join(' and ');
}

export async function fetchValueListRows() {
  const prop = _vlActiveProperty;
  const vl = _vlActiveValueList;
  const servicePath = _vlActiveServicePath;
  if (!prop || !vl || !servicePath) return;
  const filter = document.getElementById('vlFilter').value.trim();
  const searchEl = document.getElementById('vlSearch');
  const searchTerm = (searchEl && !searchEl.classList.contains('hidden'))
    ? searchEl.value.trim()
    : '';
  const top = parseInt(document.getElementById('vlTop').value, 10) || 100;
  const status = document.getElementById('vlStatus');
  const results = document.getElementById('vlResults');
  status.textContent = 'Fetching…';
  results.innerHTML = '<div class="p-4 text-ox-dim text-[11px]">Loading…</div>';
  try {
    const params = {
      entity_set: vl.collection_path,
      filter: filter || null,
      search: searchTerm || null,
      top,
    };
    const data = await timedInvoke('run_query', {
      profileName: state.currentProfile,
      servicePath,
      params,
    });
    renderValueListRows(data, prop, vl);
  } catch (e) {
    status.textContent = 'Fetch error';
    results.innerHTML = safeHtml`<div class="p-4 text-ox-red text-[11px]">${String(e)}</div>`;
  }
}

function renderValueListRows(data, prop, vl) {
  const rows = extractRows(data) || [];
  const status = document.getElementById('vlStatus');
  const results = document.getElementById('vlResults');
  status.textContent = `${rows.length} row(s).`;
  if (rows.length === 0) {
    results.innerHTML = '<div class="p-4 text-ox-dim text-[11px]">No rows.</div>';
    return;
  }
  // Column order: prioritize ValueListProperty names from the mapping
  // so the picker feels task-shaped; then fill in any remaining keys.
  void prop;
  const priority = vl.parameters.map(p => p.value_list_property);
  const firstRowKeys = Object.keys(rows[0]).filter(k => !k.startsWith('__'));
  const cols = [];
  for (const k of priority) if (firstRowKeys.includes(k) && !cols.includes(k)) cols.push(k);
  for (const k of firstRowKeys) if (!cols.includes(k)) cols.push(k);
  let html = '<table class="w-full"><thead class="sticky top-0 bg-ox-surface z-10"><tr class="text-ox-dim text-[10px]">';
  for (const c of cols) {
    html += `<th class="text-left px-3 py-1 border-b border-ox-border">${escapeHtml(c)}</th>`;
  }
  html += '</tr></thead><tbody>';
  for (let i = 0; i < rows.length; i++) {
    html += `<tr class="hover:bg-ox-electric/10 cursor-pointer border-b border-ox-border/30" data-action="vl-pick" data-row="${i}">`;
    for (const c of cols) {
      const v = rows[i][c];
      const shown = v === null || v === undefined ? '' : String(v);
      html += `<td class="px-3 py-1 text-ox-text">${escapeHtml(shown)}</td>`;
    }
    html += '</tr>';
  }
  html += '</tbody></table>';
  results.innerHTML = html;
  // Stash rows on the container so the pick handler can read without
  // re-fetching. Use a private key to avoid clashing with user data.
  results._vlRows = rows;
}

export function pickValueListRow(rowIndex) {
  const prop = _vlActiveProperty;
  const vl = _vlActiveValueList;
  const results = document.getElementById('vlResults');
  if (!prop || !vl || !results._vlRows) return;
  const row = results._vlRows[rowIndex];
  if (!row) return;
  const tab = getActiveTab();
  const info = tab && tab._lastDescribeInfo;
  if (!info) return;
  const clauses = [];
  for (const param of vl.parameters) {
    // InOut and Out bind picked VL values back onto local properties.
    // In parameters are one-way (local → VL) and don't get written back.
    if (param.kind !== 'inout' && param.kind !== 'out') continue;
    if (!param.local_property) continue;
    const value = row[param.value_list_property];
    if (value === null || value === undefined) continue;
    const localProp = info.properties.find(p => p.name === param.local_property);
    const lit = formatODataLiteral(value, localProp ? localProp.edm_type : 'Edm.String');
    clauses.push(`${param.local_property} eq ${lit}`);
  }
  if (clauses.length === 0) {
    closeValueListPicker();
    return;
  }
  // Merge into main $filter with `and`. If any of our clauses is
  // already present verbatim, drop it — this matters on the common
  // "user opens picker, picks same row twice" case.
  const filterInput = document.getElementById('qFilter');
  const existing = (filterInput.value || '').trim();
  const merged = existing
    ? [existing, ...clauses.filter(c => !existing.includes(c))].join(' and ')
    : clauses.join(' and ');
  filterInput.value = merged;
  closeValueListPicker();
  filterInput.focus();
}
