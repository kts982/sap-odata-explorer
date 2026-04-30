// ── Module imports ──
// `invoke` is for the small handful of commands that don't go through
// `timedInvoke` (auth/profile lifecycle — no spinner / trace correlation
// needed). Vendored copy of @tauri-apps/api/core (see
// scripts/vendor-tauri.cjs); direct relative import avoids needing an
// importmap under the locked-down `script-src 'self'` CSP. Re-run
// `npm run vendor:tauri` after bumping @tauri-apps/api in package.json.
import { invoke } from './vendor/tauri-core.js';
import { state } from './state.js';
import {
  tabScope,
  timedInvoke,
  applyTraceToTab,
  updateServicePathBar,
} from './api.js';
import {
  getTab,
  getActiveTab,
  addTab,
  closeTab,
  switchTab,
  renderTabBar,
  saveCurrentTabState,
} from './tabs.js';
import {
  getProfileMeta,
  isBrowserAuthProfile,
  updateProfileAuthUi,
  signOutCurrentProfile,
  removeCurrentProfile,
  signInCurrentProfile,
  browserAuthMessage,
} from './auth.js';
import { getFavorites, toggleFavorite } from './favorites.js';
import {
  loadProfiles,
  loadService,
  searchServices,
  pickService,
  resolveAndLoadService,
  selectEntity,
  resetResultsArea,
  renderFavoritesOnlySidebar,
} from './services.js';
import { escapeHtml, safeHtml, raw } from './html.js';
import {
  criticalityDot,
  formatDisplayValue,
  formatODataLiteral,
  valueListSummary,
  valueListHint,
  criticalityHint,
} from './format.js';
import { setStatus, setTime, showSpinner, hideSpinner } from './status.js';

export function restoreTabUI() {
  const tab = getActiveTab();
  if (!tab) return;

  // Sync global convenience vars (used in some legacy fn calls)
  state.currentProfile    = tab.profile;
  state.currentServicePath = tab.servicePath;
  state.currentEntitySet  = tab.entitySet;
  state.entitySets        = tab.entitySets;
  state.cachedServices    = tab.cachedServices;
  state.lastSearchQuery   = tab.lastSearchQuery;
  state.expandedDataStore = tab._expandedDataStore || {};
  state.lastResultRows = tab._lastResultRows || null;

  // Sync profile dropdown
  document.getElementById('profileSelect').value = state.currentProfile || '';
  updateProfileAuthUi(state.currentProfile);

  // Service path bar
  updateServicePathBar(tab);

  // Service input
  document.getElementById('serviceInput').value = tab._serviceInput || '';

  // Sidebar
  document.getElementById('sidebarTitle').textContent = tab._sidebarTitle || 'Services';
  document.getElementById('sidebarCount').textContent = tab._sidebarCount || '';
  if (tab._sidebarHtml !== undefined) {
    document.getElementById('entityList').innerHTML = tab._sidebarHtml;
    // Re-attach sidebar item click handlers (lost when innerHTML was set)
    reattachSidebarHandlers();
  } else {
    document.getElementById('entityList').innerHTML =
      '<div class="px-4 py-8 text-center"><div class="text-ox-dim text-xs">Select a profile and search</div></div>';
  }

  // Describe panel
  if (tab._describePanelHidden === false) {
    document.getElementById('describePanel').classList.remove('hidden');
    document.getElementById('entityTitle').textContent = tab._describeTitle || '';
    document.getElementById('describeContent').innerHTML = tab._describeHtml || '';
  } else {
    document.getElementById('describePanel').classList.add('hidden');
  }

  // Query bar
  if (tab._queryBarHidden === false) {
    document.getElementById('queryBar').classList.remove('hidden');
    document.getElementById('queryEntitySet').textContent = tab._queryEntitySet || '';
    document.getElementById('qSelect').value  = tab._qSelect  || '';
    document.getElementById('qFilter').value  = tab._qFilter  || '';
    document.getElementById('qExpand').value  = tab._qExpand  || '';
    document.getElementById('qOrderby').value = tab._qOrderby || '';
    document.getElementById('qTop').value     = tab._qTop     !== undefined ? tab._qTop : '20';
    document.getElementById('qSkip').value    = tab._qSkip    || '';
  } else {
    document.getElementById('queryBar').classList.add('hidden');
    document.getElementById('qSelect').value  = '';
    document.getElementById('qFilter').value  = '';
    document.getElementById('qExpand').value  = '';
    document.getElementById('qOrderby').value = '';
    document.getElementById('qTop').value     = '20';
    document.getElementById('qSkip').value    = '';
  }

  // History panel
  if (tab._historyVisible) {
    renderHistoryPanel(tab);
    document.getElementById('historyPanel').classList.remove('hidden');
  } else {
    document.getElementById('historyPanel').classList.add('hidden');
  }

  renderTraceSummary(tab);
  if (tab._traceVisible) {
    renderTraceInspector(tab);
    document.getElementById('traceInspectorPanel').classList.remove('hidden');
  } else {
    document.getElementById('traceInspectorPanel').classList.add('hidden');
  }
  updateTraceToggleState(tab._traceVisible);

  renderAnnotationBadge(tab.annotationSummary);

  // Stats bar
  if (tab._statsVisible) {
    document.getElementById('statRows').textContent = tab._statRows || '';
    document.getElementById('statSize').textContent = tab._statSize || '';
    document.getElementById('statTiming').innerHTML = tab._statTiming || '';
    document.getElementById('statsBar').classList.remove('hidden');
  } else {
    document.getElementById('statsBar').classList.add('hidden');
  }

  // Results
  if (tab._resultsHtml !== undefined) {
    document.getElementById('resultsArea').innerHTML = tab._resultsHtml;
  } else {
    resetResultsArea();
  }
}

/** Re-attach event handlers on sidebar items after innerHTML restore */
function reattachSidebarHandlers() {
  // All sidebar items (back link, service items, star buttons, entity items)
  // are handled by document-level delegation only — nothing to re-attach.
}

document.getElementById('profileSelect').addEventListener('change', (e) => {
  const profile = e.target.value || null;

  saveCurrentTabState();
  const tab = getActiveTab();
  if (!tab) return;

  tab.profile = profile;
  tab.servicePath = null;
  tab.serviceVersion = null;
  tab.entitySet = null;
  tab.entitySets = [];
  tab.cachedServices = null;
  tab.lastSearchQuery = null;
  tab._sidebarHtml = undefined;
  tab._sidebarTitle = 'Services';
  tab._sidebarCount = '';
  tab._serviceInput = '';
  tab._queryBarHidden = true;
  tab._describePanelHidden = true;
  tab._statsVisible = false;
  tab._resultsHtml = undefined;
  tab._historyVisible = false;
  tab.httpTraceEntries = [];
  tab.selectedTraceId = null;
  tab._traceVisible = false;
  tab.annotationSummary = null;

  // Sync globals
  state.currentProfile = profile;
  state.currentServicePath = null;
  state.currentEntitySet = null;
  state.entitySets = [];
  state.cachedServices = null;
  state.lastSearchQuery = null;

  document.getElementById('entityList').innerHTML =
    '<div class="px-4 py-8 text-center"><div class="text-ox-dim text-xs">Click Search to browse services</div></div>';
  document.getElementById('queryBar').classList.add('hidden');
  document.getElementById('describePanel').classList.add('hidden');
  document.getElementById('statsBar').classList.add('hidden');
  document.getElementById('historyPanel').classList.add('hidden');
  document.getElementById('traceInspectorPanel').classList.add('hidden');
  updateTraceToggleState(false);
  renderAnnotationBadge(null);
  document.getElementById('serviceInput').value = '';
  document.getElementById('sidebarTitle').textContent = 'Services';
  document.getElementById('sidebarCount').textContent = '';
  updateServicePathBar(null);
  resetResultsArea();
  renderTraceSummary(tab);
  updateProfileAuthUi(profile);

  if (profile) {
    setStatus(`Connected to ${profile}`);
    // If this profile has favorites stored locally, render them immediately
    // from localStorage — no catalog fetch. Search still populates the full list.
    if (getFavorites(profile).length > 0) {
      renderFavoritesOnlySidebar(profile);
    }
  }
});

// ══════════════════════════════════════════════════════════════
// ── DESCRIBE PANEL ──
// ══════════════════════════════════════════════════════════════

// SAP view helper: render small pills for property-level restrictions.
// We only surface *deviations from the default* — filterable/sortable/
// creatable/updatable are normally true, so showing "no filter", "no sort",
// "read-only" is what's informative. required_in_filter=true is also
// visible because it constrains how the user writes $filter.
function propertyFlagHints(p) {
  const badges = [];
  if (p.filterable === false) {
    badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="sap:filterable=false — server rejects $filter on this column">no filter</span>`);
  }
  if (p.sortable === false) {
    badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="sap:sortable=false — server rejects $orderby on this column">no sort</span>`);
  }
  if (p.creatable === false && p.updatable === false) {
    badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="sap:creatable=false and sap:updatable=false — server assigns this value, clients cannot write it">read-only</span>`);
  } else {
    if (p.creatable === false) {
      badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="sap:creatable=false">no create</span>`);
    }
    if (p.updatable === false) {
      badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="sap:updatable=false">no update</span>`);
    }
  }
  if (p.required_in_filter === true) {
    badges.push(`<span class="text-[9px] text-ox-amber bg-ox-amberGlow border border-ox-amber/40 rounded-sm px-1 py-px" title="sap:required-in-filter=true — the server requires $filter to constrain this column">req.filter</span>`);
  }
  // Common.FieldControl — write/display control. Mandatory overlaps
  // semantically with required_in_filter so we keep the pills distinct
  // (one is $filter-side, the other is write-side). ReadOnly overlaps
  // with updatable=false; suppress the pill when we'd double-count.
  if (p.field_control) {
    const fc = p.field_control;
    if (fc.kind === 'mandatory') {
      badges.push(`<span class="text-[9px] text-ox-amber bg-ox-amberGlow border border-ox-amber/40 rounded-sm px-1 py-px" title="Common.FieldControl=Mandatory — required on write">mandatory</span>`);
    } else if (fc.kind === 'readonly' && !(p.updatable === false && p.creatable === false)) {
      badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="Common.FieldControl=ReadOnly">read-only</span>`);
    } else if (fc.kind === 'inapplicable') {
      badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="Common.FieldControl=Inapplicable — not relevant for this record">n/a</span>`);
    } else if (fc.kind === 'hidden') {
      badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="Common.FieldControl=Hidden">hidden</span>`);
    } else if (fc.kind === 'path') {
      badges.push(`<span class="text-[9px] text-ox-blue border border-ox-blue/40 rounded-sm px-1 py-px" title="Common.FieldControl Path — state driven by ${escapeHtml(fc.value)} at runtime">⇨ ${escapeHtml(fc.value)}</span>`);
    }
    // `optional` is the default; no pill needed.
  }
  // UI.Hidden / UI.HiddenFilter — marker pills.
  if (p.hidden && (!p.field_control || p.field_control.kind !== 'hidden')) {
    badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="UI.Hidden — Fiori would not show this property">UI hidden</span>`);
  }
  if (p.hidden_filter) {
    badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="UI.HiddenFilter — shown as a column but suppressed from Fiori's filter bar">no filter UI</span>`);
  }
  // V2 sap:display-format — presentation hint. Small-caps pill.
  if (p.display_format) {
    const val = p.display_format;
    badges.push(`<span class="text-[9px] text-ox-green border border-ox-green/40 rounded-sm px-1 py-px" title="sap:display-format=${escapeHtml(val)}">fmt: ${escapeHtml(val)}</span>`);
  }
  // Common.SemanticObject — Fiori cross-app navigation target.
  if (p.semantic_object) {
    badges.push(`<span class="text-[9px] text-ox-blue border border-ox-blue/40 rounded-sm px-1 py-px" title="Common.SemanticObject — Fiori cross-app navigation target">&#8605; ${escapeHtml(p.semantic_object)}</span>`);
  }
  // Common.Masked — sensitive data warning.
  if (p.masked) {
    badges.push(`<span class="text-[9px] text-ox-amber bg-ox-amberGlow border border-ox-amber/40 rounded-sm px-1 py-px" title="Common.Masked — sensitive / PII data; Fiori masks the value at runtime">masked</span>`);
  }
  return badges.length ? ' ' + badges.join(' ') : '';
}

// Extract identifier-looking tokens from a free-form OData expression.
// Good enough for cross-checking against known property names — any
// identifier that happens to appear in the expression AND matches a
// restricted property name flags as a likely reference.
function extractIdentifiers(text) {
  if (!text) return [];
  return (text.match(/[A-Za-z_][A-Za-z0-9_]*/g) || []);
}

// Pre-flight validator for SAP View. Returns a list of human-readable
// restriction violations — empty list means "OK to run".
function validateQueryRestrictions(params, info) {
  if (!info || !Array.isArray(info.properties)) return [];
  const issues = [];
  const byName = new Map(info.properties.map(p => [p.name, p]));

  if (params.filter) {
    const tokens = new Set(extractIdentifiers(params.filter));
    for (const p of info.properties) {
      if (p.filterable === false && tokens.has(p.name)) {
        issues.push(`'${p.name}' is non-filterable (Capabilities.FilterRestrictions / sap:filterable=false) but referenced in $filter.`);
      }
    }
  }

  if (params.orderby) {
    const tokens = new Set(extractIdentifiers(params.orderby));
    for (const p of info.properties) {
      if (p.sortable === false && tokens.has(p.name)) {
        issues.push(`'${p.name}' is non-sortable but referenced in $orderby.`);
      }
    }
  }

  // required_in_filter: these properties MUST be narrowed in $filter.
  const required = info.properties.filter(p => p.required_in_filter === true);
  if (required.length) {
    const tokens = new Set(extractIdentifiers(params.filter || ''));
    for (const p of required) {
      if (!tokens.has(p.name)) {
        issues.push(`'${p.name}' requires a filter clause (Capabilities.FilterRestrictions.RequiredProperties / sap:required-in-filter).`);
      }
    }
  }

  // Entity-set-level capabilities — the server will 500 on these if
  // the declared flag is explicitly false. Only the `false` case is
  // informative; `None`/unset defaults to "supported".
  if ((params.top !== null && params.top !== undefined && params.top !== '') && info.top_supported === false) {
    issues.push('$top was set, but Capabilities.TopSupported=false on this set — the server will reject pagination.');
  }
  if ((params.skip !== null && params.skip !== undefined && params.skip !== '') && info.skip_supported === false) {
    issues.push('$skip was set, but Capabilities.SkipSupported=false on this set — the server will reject pagination.');
  }
  if (params.count === true && info.countable === false) {
    issues.push('$count requested, but Capabilities.CountRestrictions.Countable=false on this set.');
  }

  // $expand: flag nav paths the service marked non-expandable. We only
  // match on the first segment (the nav prop directly on the annotated
  // entity) — multi-hop bans would require walking the type graph.
  if (params.expand && Array.isArray(info.non_expandable_properties) && info.non_expandable_properties.length) {
    const expandRoots = new Set(
      params.expand.split(',').map(s => s.trim().split('/')[0]).filter(Boolean)
    );
    for (const np of info.non_expandable_properties) {
      if (expandRoots.has(np)) {
        issues.push(`'${np}' is listed in Capabilities.ExpandRestrictions.NonExpandableProperties but referenced in $expand.`);
      }
    }
  }
  if (params.expand && info.expandable === false) {
    issues.push('$expand requested, but Capabilities.ExpandRestrictions.Expandable=false — the set rejects expansion entirely.');
  }

  // byName lookup silences unused warnings; also handy for future checks.
  void byName;
  return issues;
}

// Show or hide the amber warnings strip above the results. An empty
// list hides it. Called by executeQuery and by the SAP-view toggle so
// stale warnings don't linger after the user flips the mode off.
function showSapViewWarnings(issues) {
  const strip = document.getElementById('sapViewWarnings');
  const list = document.getElementById('sapViewWarningsList');
  if (!strip || !list) return;
  if (!issues || issues.length === 0) {
    strip.classList.add('hidden');
    list.textContent = '';
    return;
  }
  strip.classList.remove('hidden');
  list.textContent = issues.map(i => `• ${i}`).join('\n');
}

// Render the "selection fields" chip bar above the query inputs. Only
// visible when SAP View is on, the entity type has UI.SelectionFields,
// and we're looking at that entity's describe panel. Clicking a chip
// seeds $filter with a skeleton clause the user can complete.
function renderSelectionFieldsBar(info) {
  const bar = document.getElementById('selectionFieldsBar');
  const host = document.getElementById('selectionFieldsChips');
  if (!bar || !host) return;
  const fields = state.sapViewEnabled && info && Array.isArray(info.selection_fields)
    ? info.selection_fields
    : [];
  if (fields.length === 0) {
    bar.classList.add('hidden');
    host.innerHTML = '';
    return;
  }
  bar.classList.remove('hidden');
  // Amber-flag chips whose backing property is required-in-filter, so the
  // user sees at a glance which selection fields the server will reject
  // queries without.
  const byName = new Map(
    Array.isArray(info.properties)
      ? info.properties.map(p => [p.name, p])
      : []
  );
  host.innerHTML = fields
    .map(name => {
      const p = byName.get(name);
      const req = p && p.required_in_filter === true;
      const cls = req
        ? 'text-[10px] px-1.5 py-0.5 rounded-sm text-ox-amber bg-ox-amberGlow border border-ox-amber/40 hover:bg-ox-amber/20'
        : 'btn-ghost text-[10px] px-1.5 py-0.5 rounded-sm';
      const tipBase = req
        ? 'Required in $filter — append and narrow'
        : 'Append to $filter';
      const title = `${tipBase}\nShift-click to append to $select instead`;
      return `<button type="button" class="${cls}" data-action="selection-field" data-name="${escapeHtml(name)}" title="${title}">${escapeHtml(name)}</button>`;
    })
    .join('');
}

// ══════════════════════════════════════════════════════════════
// ── SELECTION FILTER BAR ──
// ══════════════════════════════════════════════════════════════
// Mock Fiori filter bar: one row per UI.SelectionFields property with
// an operator dropdown + value input. Apply builds a $filter clause
// and drops it into the query bar's qFilter. Useful when the user
// wants to fill several selection fields without memorizing OData
// operator syntax (startswith/contains/between).

function openFilterBar() {
  const tab = getActiveTab();
  const info = tab && tab._lastDescribeInfo;
  if (!info || !Array.isArray(info.selection_fields) || info.selection_fields.length === 0) {
    setStatus('This entity declares no UI.SelectionFields.');
    return;
  }
  const modal = document.getElementById('filterBarModal');
  const rowsHost = document.getElementById('fbRows');
  const subtitle = document.getElementById('fbSubtitle');
  subtitle.textContent = info.name;
  rowsHost.innerHTML = buildFilterBarRows(info);
  modal.classList.remove('hidden');
  // Focus the first value input.
  setTimeout(() => {
    const first = rowsHost.querySelector('input[data-fb="value"]');
    if (first) first.focus();
  }, 0);
}

function closeFilterBar() {
  document.getElementById('filterBarModal').classList.add('hidden');
}

// Render one row per SelectionField. Each row is a flex with label,
// operator dropdown, value input, and (for `between`) a second value
// input. Operators cover strings, numbers, and dates; we don't filter
// by type because SAP's CDS sometimes annotates columns in ways that
// make the "right" set hard to guess.
function buildFilterBarRows(info) {
  const propByName = new Map(info.properties.map(p => [p.name, p]));
  const operators = [
    ['eq', '='], ['ne', '≠'],
    ['gt', '>'], ['ge', '≥'], ['lt', '<'], ['le', '≤'],
    ['contains', 'contains'],
    ['startswith', 'starts with'], ['endswith', 'ends with'],
    ['between', 'between'],
  ];
  const opOptions = operators
    .map(([v, label]) => `<option value="${v}">${label}</option>`)
    .join('');
  return info.selection_fields.map(name => {
    const p = propByName.get(name);
    const type = p ? p.edm_type.replace('Edm.', '') : '';
    const req = p && p.required_in_filter === true;
    const reqBadge = req
      ? `<span class="text-[9px] text-ox-amber bg-ox-amberGlow border border-ox-amber/40 rounded-sm px-1 ml-1" title="required-in-filter">req</span>`
      : '';
    return `
      <div class="grid grid-cols-[160px_90px_1fr_auto] gap-2 items-center" data-fb-row data-field="${escapeHtml(name)}">
        <div class="text-[11px] text-ox-text truncate" title="${escapeHtml(name)} (${escapeHtml(type)})">${escapeHtml(name)}${reqBadge} <span class="text-ox-dim">${escapeHtml(type)}</span></div>
        <select data-fb="op" class="bg-ox-surface text-ox-text text-[11px] font-mono border border-ox-border rounded-sm px-1.5 py-1 outline-hidden">${opOptions}</select>
        <div data-fb="inputs" class="flex items-center gap-1">
          <input data-fb="value" type="text" placeholder="value"
            class="flex-1 bg-ox-surface text-ox-text text-xs font-mono border border-ox-border rounded-sm px-2 py-1 outline-hidden" />
        </div>
        <button type="button" data-action="fb-clear-row" class="btn-ghost text-[10px] px-1.5 py-0.5 rounded-sm" title="Clear this row">×</button>
      </div>`;
  }).join('');
}

function resetFilterBar() {
  const host = document.getElementById('fbRows');
  host.querySelectorAll('input[data-fb="value"], input[data-fb="value-high"]').forEach(i => i.value = '');
  host.querySelectorAll('select[data-fb="op"]').forEach(s => s.value = 'eq');
  // Remove any "high" inputs that may have been added by between-operator.
  host.querySelectorAll('[data-fb="value-high"]').forEach(el => el.remove());
}

// Switch the row's inputs area to show a second value when operator
// is `between`. Revert to single input for anything else.
function onFilterBarOpChange(selectEl) {
  const row = selectEl.closest('[data-fb-row]');
  if (!row) return;
  const inputs = row.querySelector('[data-fb="inputs"]');
  const op = selectEl.value;
  const hasHigh = inputs.querySelector('[data-fb="value-high"]');
  if (op === 'between' && !hasHigh) {
    const sep = document.createElement('span');
    sep.className = 'text-ox-dim text-[10px]';
    sep.textContent = 'and';
    sep.setAttribute('data-fb', 'value-high-sep');
    const high = document.createElement('input');
    high.type = 'text';
    high.placeholder = 'upper';
    high.className = 'flex-1 bg-ox-surface text-ox-text text-xs font-mono border border-ox-border rounded-sm px-2 py-1 outline-hidden';
    high.setAttribute('data-fb', 'value-high');
    inputs.appendChild(sep);
    inputs.appendChild(high);
  } else if (op !== 'between' && hasHigh) {
    inputs.querySelectorAll('[data-fb="value-high"], [data-fb="value-high-sep"]').forEach(el => el.remove());
  }
}

// Walk the filter-bar rows and build an OData $filter expression.
// Empty rows are skipped. Literals are quoted via formatODataLiteral
// using the property's edm_type. Clauses are joined with `and`.
function buildFilterBarExpression() {
  const tab = getActiveTab();
  const info = tab && tab._lastDescribeInfo;
  if (!info) return '';
  const propByName = new Map(info.properties.map(p => [p.name, p]));
  const clauses = [];
  const rows = document.querySelectorAll('#fbRows [data-fb-row]');
  rows.forEach(row => {
    const name = row.dataset.field;
    const prop = propByName.get(name);
    if (!prop) return;
    const op = row.querySelector('[data-fb="op"]').value;
    const val = row.querySelector('[data-fb="value"]').value.trim();
    if (val === '') return;
    const lit = formatODataLiteral(val, prop.edm_type);
    let clause;
    switch (op) {
      case 'eq': case 'ne': case 'gt': case 'ge': case 'lt': case 'le':
        clause = `${name} ${op} ${lit}`;
        break;
      case 'contains':
        clause = `contains(${name},${lit})`;
        break;
      case 'startswith':
        clause = `startswith(${name},${lit})`;
        break;
      case 'endswith':
        clause = `endswith(${name},${lit})`;
        break;
      case 'between': {
        const high = row.querySelector('[data-fb="value-high"]');
        const highVal = high ? high.value.trim() : '';
        if (!highVal) {
          clause = `${name} ge ${lit}`; // degraded form — no upper bound given
        } else {
          const highLit = formatODataLiteral(highVal, prop.edm_type);
          clause = `(${name} ge ${lit} and ${name} le ${highLit})`;
        }
        break;
      }
      default:
        return;
    }
    clauses.push(clause);
  });
  return clauses.join(' and ');
}

function applyFilterBar() {
  const expr = buildFilterBarExpression();
  if (!expr) {
    setStatus('No filter rows with values — nothing applied.');
    return;
  }
  document.getElementById('qFilter').value = expr;
  closeFilterBar();
  document.getElementById('qFilter').focus();
}

// ══════════════════════════════════════════════════════════════
// ── VALUE-LIST PICKER ──
// ══════════════════════════════════════════════════════════════
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

async function openValueListPicker(propertyName) {
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
async function selectVariant(index) {
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

function closeValueListPicker() {
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

async function fetchValueListRows() {
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

function pickValueListRow(rowIndex) {
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

// Show the "Fiori cols" button when SAP View is on and UI.LineItem is
// present. Filter to DataFields whose value_path is an actual property
// on this entity so V4 $select stays valid — nav-path DataFields are
// skipped (they belong in $expand).
function renderFioriColsButton(info) {
  const btn = document.getElementById('btnFioriCols');
  if (!btn) return;
  const active = state.sapViewEnabled && info;
  const fields = active && Array.isArray(info.line_item) ? info.line_item : [];
  const propNames = new Set(
    Array.isArray(info && info.properties) ? info.properties.map(p => p.name) : []
  );
  // LineItem value_paths (the visible columns Fiori would show).
  const linePaths = fields
    .map(f => f && f.value_path)
    .filter(p => typeof p === 'string' && propNames.has(p));
  // PresentationVariant.RequestAtLeast (the silent "always include" set
  // — time zones, description keys, etc.). Appended AFTER the LineItem
  // columns so they don't disturb the positional order a Fiori list
  // report would use. Dedupe against line_item to avoid double-listing.
  const lineSet = new Set(linePaths);
  const requestAtLeast = active && Array.isArray(info.request_at_least)
    ? info.request_at_least.filter(p => propNames.has(p) && !lineSet.has(p))
    : [];
  const paths = [...linePaths, ...requestAtLeast];
  if (paths.length === 0) {
    btn.classList.add('hidden');
    btn.removeAttribute('data-paths');
    btn.removeAttribute('data-orderby');
    return;
  }
  btn.classList.remove('hidden');
  // Label notes the RequestAtLeast augment when it kicked in so the
  // user understands why the select is wider than the visible cols.
  const suffix = requestAtLeast.length ? ` +${requestAtLeast.length}` : '';
  btn.textContent = `Fiori cols (${linePaths.length}${suffix})`;
  // UI.PresentationVariant.SortOrder → "$orderby" string. Built here so
  // the click handler can drop it in without re-reading the describe
  // info. Only direct properties survive (nav-path sorts would need
  // $expand gymnastics).
  const sortOrder = active && Array.isArray(info.sort_order) ? info.sort_order : [];
  const orderbyClauses = sortOrder
    .filter(s => s && typeof s.property === 'string' && propNames.has(s.property))
    .map(s => `${s.property} ${s.descending ? 'desc' : 'asc'}`);
  const orderbyStr = orderbyClauses.join(',');
  const tipBase = 'Populate $select with UI.LineItem default columns (Fiori list report).';
  const tipLines = [tipBase];
  if (requestAtLeast.length) {
    tipLines.push(`Includes ${requestAtLeast.length} UI.PresentationVariant.RequestAtLeast field(s).`);
  }
  if (orderbyStr) {
    tipLines.push(`Also sets $orderby: ${orderbyStr}`);
  }
  btn.title = tipLines.join('\n');
  btn.dataset.paths = paths.join(',');
  btn.dataset.orderby = orderbyStr;
}

// Show the "Fiori filter" button when SAP View is on and the entity
// has at least one UI.SelectionVariant declared. We use the default
// (no-qualifier) variant for the actual filter build; the button's
// label hints at the qualified-variant count so the user knows extras
// exist (multi-variant picker UI can come later).
function renderFioriFilterButton(info) {
  const btn = document.getElementById('btnFioriFilter');
  if (!btn) return;
  const variants = state.sapViewEnabled && info && Array.isArray(info.selection_variants)
    ? info.selection_variants
    : [];
  if (variants.length === 0) {
    btn.classList.add('hidden');
    btn.removeAttribute('data-variant-index');
    return;
  }
  // Pick the first variant that actually builds a non-empty $filter.
  // Services sometimes declare an empty "default view" variant (e.g.
  // "Show All") first and only put real filter clauses on qualified
  // variants — preferring a populated one gives the user a useful
  // result on the single click.
  let chosenIndex = -1;
  let chosenClause = '';
  for (let i = 0; i < variants.length; i++) {
    const clause = buildSelectionVariantFilter(variants[i], info);
    if (clause) {
      chosenIndex = i;
      chosenClause = clause;
      break;
    }
  }
  if (chosenIndex === -1) {
    // None of the variants produced an actionable filter — hide
    // rather than offer a dead button.
    btn.classList.add('hidden');
    return;
  }
  const variant = variants[chosenIndex];
  btn.classList.remove('hidden');
  const totalCount = variants.length;
  const label = variant.text
    ? `Fiori filter: ${variant.text}`
    : 'Fiori filter';
  const suffix = totalCount > 1 ? ` (+${totalCount - 1})` : '';
  btn.textContent = label + suffix;
  const preview = chosenClause.length > 80 ? chosenClause.slice(0, 80) + '…' : chosenClause;
  const extraLines = [];
  if (totalCount > 1) {
    extraLines.push(`${totalCount - 1} additional variant(s) not yet exposed in a picker.`);
  }
  if (variant.qualifier) {
    extraLines.push(`Qualifier: ${variant.qualifier}`);
  }
  btn.title = `$filter ← ${preview}${extraLines.length ? '\n\n' + extraLines.join('\n') : ''}`;
  btn.dataset.variantIndex = String(chosenIndex);
}

// Replace $filter with the clause built from a UI.SelectionVariant.
// Overwrites rather than merges (same reasoning as "Fiori cols" — the
// action's meaning is "show me this variant's filter as-is"). Uses the
// variant index stashed by renderFioriFilterButton so an empty leading
// variant doesn't get picked over a populated qualified one.
function applyFioriFilter() {
  const tab = getActiveTab();
  const info = tab && tab._lastDescribeInfo;
  if (!info || !Array.isArray(info.selection_variants) || info.selection_variants.length === 0) return;
  const btn = document.getElementById('btnFioriFilter');
  const idx = btn && btn.dataset.variantIndex
    ? parseInt(btn.dataset.variantIndex, 10)
    : 0;
  const variant = info.selection_variants[idx] || info.selection_variants[0];
  const clause = buildSelectionVariantFilter(variant, info);
  if (!clause) return;
  const input = document.getElementById('qFilter');
  input.value = clause;
  input.focus();
}

// Convert a SelectionVariant into an OData $filter expression:
//   - Parameters become `name eq <lit>` clauses (AND-joined with the rest).
//   - Each SelectOption becomes an OR-joined group of range clauses for
//     that property, optionally wrapped in `not (...)` for sign=E.
//   - Properties are AND-joined overall.
// Returns an empty string when the variant has no usable clauses.
function buildSelectionVariantFilter(variant, info) {
  if (!variant) return '';
  const propByName = new Map(
    Array.isArray(info && info.properties) ? info.properties.map(p => [p.name, p]) : []
  );
  const andParts = [];
  for (const param of variant.parameters || []) {
    const prop = propByName.get(param.property_name);
    if (!prop) continue;
    const lit = formatODataLiteral(param.property_value, prop.edm_type);
    andParts.push(`${param.property_name} eq ${lit}`);
  }
  for (const opt of variant.select_options || []) {
    const prop = propByName.get(opt.property_name);
    if (!prop) continue;
    const rangeClauses = [];
    for (const range of opt.ranges || []) {
      const clause = rangeToFilter(prop, range);
      if (clause) rangeClauses.push(clause);
    }
    if (rangeClauses.length === 0) continue;
    const combined = rangeClauses.length === 1
      ? rangeClauses[0]
      : `(${rangeClauses.join(' or ')})`;
    andParts.push(combined);
  }
  return andParts.join(' and ');
}

// One SelectionRange → one OData filter clause. Handles the seven
// common operators; CP/NP (SAP SELECT-OPTIONS pattern matching with *)
// are skipped because their OData translation depends on server-side
// substringof/contains support and the wildcard syntax mismatch — not
// worth mis-rendering for an MVP.
function rangeToFilter(prop, range) {
  const lit = formatODataLiteral(range.low, prop.edm_type);
  const name = prop.name;
  let clause;
  switch (range.option) {
    case 'eq': clause = `${name} eq ${lit}`; break;
    case 'ne': clause = `${name} ne ${lit}`; break;
    case 'gt': clause = `${name} gt ${lit}`; break;
    case 'ge': clause = `${name} ge ${lit}`; break;
    case 'lt': clause = `${name} lt ${lit}`; break;
    case 'le': clause = `${name} le ${lit}`; break;
    case 'bt': {
      if (range.high === null || range.high === undefined) return '';
      const hi = formatODataLiteral(range.high, prop.edm_type);
      clause = `(${name} ge ${lit} and ${name} le ${hi})`;
      break;
    }
    case 'nb': {
      if (range.high === null || range.high === undefined) return '';
      const hi = formatODataLiteral(range.high, prop.edm_type);
      // Invert BT: outside the closed interval.
      clause = `(${name} lt ${lit} or ${name} gt ${hi})`;
      break;
    }
    default:
      // CP/NP and anything unknown — skip rather than guess.
      return '';
  }
  return range.sign === 'e' ? `not (${clause})` : clause;
}

// Replace $select (and $orderby if the service declares one) with the
// Fiori LineItem defaults. Overwrites rather than appends — "show me
// what Fiori shows" means both the column list and the default sort.
function applyFioriCols() {
  const btn = document.getElementById('btnFioriCols');
  const input = document.getElementById('qSelect');
  if (!btn || !input) return;
  const paths = (btn.dataset.paths || '').split(',').filter(Boolean);
  if (paths.length === 0) return;
  input.value = paths.join(',');
  // UI.PresentationVariant.SortOrder → $orderby. Stashed on the button
  // as `data-orderby` by renderFioriColsButton so the click is a pure
  // apply step with no DOM lookups.
  const orderby = btn.dataset.orderby || '';
  const orderbyInput = document.getElementById('qOrderby');
  if (orderbyInput) orderbyInput.value = orderby;
  input.focus();
}

// Fiori-readiness checklist, rendered below the describe tables when
// SAP View is on. Shows the parser's findings with a traffic-light
// dot per row and groups them by category. Summary counts up-front
// so the user can tell "is this service shaped like Fiori expects?"
// at a glance.
function renderFioriReadinessPanel(info) {
  const findings = Array.isArray(info.fiori_readiness) ? info.fiori_readiness : [];
  if (findings.length === 0) return '';
  const counts = { pass: 0, warn: 0, miss: 0 };
  for (const f of findings) {
    if (counts[f.severity] !== undefined) counts[f.severity]++;
  }
  const summary = [
    counts.pass ? `<span class="text-ox-green">&#9679; ${counts.pass} pass</span>` : '',
    counts.warn ? `<span class="text-ox-amber">&#9679; ${counts.warn} warn</span>` : '',
    counts.miss ? `<span class="text-ox-red">&#9679; ${counts.miss} miss</span>` : '',
  ].filter(Boolean).join(' <span class="text-ox-border">·</span> ');
  // Group by category, preserve original order within each group.
  const order = ['profile', 'identity', 'listreport', 'filtering', 'fields', 'integrity', 'capabilities'];
  const byCategory = new Map(order.map(k => [k, []]));
  for (const f of findings) {
    if (!byCategory.has(f.category)) byCategory.set(f.category, []);
    byCategory.get(f.category).push(f);
  }
  const pretty = {
    profile: 'Profile',
    identity: 'Identity',
    listreport: 'List report',
    filtering: 'Filtering',
    fields: 'Fields',
    integrity: 'Integrity',
    capabilities: 'Capabilities',
  };
  let html = '<div class="mt-4 border border-ox-border rounded-sm overflow-hidden">';
  html += `<div class="px-3 py-1.5 bg-ox-panel text-[10px] uppercase tracking-widest text-ox-dim flex items-center gap-3">`;
  html += `<span class="font-medium">Fiori readiness</span>`;
  html += `<span class="text-[10px] normal-case tracking-normal">${summary}</span>`;
  html += `</div>`;
  for (const [cat, items] of byCategory) {
    if (!items || items.length === 0) continue;
    html += `<div class="px-3 py-1 bg-ox-surface/40 text-[9px] uppercase tracking-widest text-ox-muted border-t border-ox-border/40">${escapeHtml(pretty[cat] || cat)}</div>`;
    for (const f of items) {
      const color = f.severity === 'pass' ? 'text-ox-green'
        : f.severity === 'warn' ? 'text-ox-amber'
        : 'text-ox-red';
      html += `<div class="px-3 py-1 border-t border-ox-border/40 flex items-start gap-2 text-[11px]">`;
      html += `<span class="${color} mt-0.5">&#9679;</span>`;
      html += `<div class="flex-1">`;
      html += `<span class="text-ox-dim font-mono">${escapeHtml(f.code)}</span> — <span class="text-ox-text">${escapeHtml(f.message)}</span>`;
      // ABAP CDS "fix hint" — surfaces the annotation to add at the
      // source so the linter teaches, not just grades. Only present
      // on actionable (warn/miss) findings; passes skip this line.
      if (f.suggested_cds || f.why_in_fiori) {
        html += `<div class="mt-1 text-[10px] text-ox-muted leading-snug">`;
        if (f.suggested_cds) {
          html += `<span class="text-ox-blue font-mono">ABAP CDS:</span> <code class="text-ox-blue">${escapeHtml(f.suggested_cds)}</code>`;
        }
        if (f.why_in_fiori) {
          html += `<div class="text-ox-dim">${escapeHtml(f.why_in_fiori)}</div>`;
        }
        html += `</div>`;
      }
      html += `</div></div>`;
    }
  }
  html += '</div>';
  return html;
}

export function renderDescribe(info) {
  const panel = document.getElementById('describePanel');
  panel.classList.remove('hidden');

  // Cache so the SAP-view toggle can re-render without another fetch.
  const tab = getActiveTab();
  if (tab) tab._lastDescribeInfo = info;

  // Selection-fields chip bar above the query inputs (SAP View only).
  renderSelectionFieldsBar(info);
  // UI.LineItem → "Fiori cols" quick-action button next to $select.
  renderFioriColsButton(info);
  // UI.SelectionVariant → "Fiori filter" button next to "Fiori cols".
  renderFioriFilterButton(info);

  // Title: technical name always; in SAP view, append the HeaderInfo
  // singular/plural so the entity reads as "WarehouseOrderType · Warehouse Order / Warehouse Orders".
  const titleEl = document.getElementById('entityTitle');
  titleEl.textContent = info.name;
  if (state.sapViewEnabled && info.header_info) {
    const hi = info.header_info;
    const parts = [];
    if (hi.type_name) parts.push(hi.type_name);
    if (hi.type_name_plural && hi.type_name_plural !== hi.type_name) parts.push(hi.type_name_plural);
    if (parts.length) {
      titleEl.innerHTML = safeHtml`${info.name}<span class="text-ox-dim ml-2">·</span><span class="text-ox-blue ml-2">${parts.join(' / ')}</span>`;
    }
  }

  let html = '<div class="grid grid-cols-1 lg:grid-cols-2 gap-4">';

  // Properties
  html += '<div class="overflow-auto"><table class="w-full text-xs font-mono"><thead><tr class="text-ox-dim">';
  html += '<th class="text-left pb-1.5 bg-ox-surface pr-3">Property</th>';
  html += '<th class="text-left pb-1.5 bg-ox-surface pr-3">Type</th>';
  html += '<th class="text-left pb-1.5 bg-ox-surface pr-3">Key</th>';
  html += '<th class="text-left pb-1.5 bg-ox-surface">Label</th>';
  html += '</tr></thead><tbody>';
  for (const p of info.properties) {
    const keyMark = p.is_key ? '<span class="text-ox-amber">&#9679;</span>' : '';
    // SAP view: surface the text-companion property ("↦ TextProp") next to
    // the property name. The arrow hints that this column has an associated
    // description field that Fiori renders together with it.
    const textHint = state.sapViewEnabled && p.text_path
      ? ` <span class="text-ox-blue text-[10px]" title="Common.Text → ${escapeHtml(p.text_path)}">&#x21A6; ${escapeHtml(p.text_path)}</span>`
      : '';
    const currencyHint = state.sapViewEnabled && p.iso_currency_path
      ? ` <span class="text-ox-green text-[10px]" title="Measures.ISOCurrency → ${escapeHtml(p.iso_currency_path)}">&curren; ${escapeHtml(p.iso_currency_path)}</span>`
      : '';
    const unitHint = state.sapViewEnabled && p.unit_path && !p.iso_currency_path
      ? ` <span class="text-ox-green text-[10px]" title="Measures.Unit / sap:unit → ${escapeHtml(p.unit_path)}">&#8593; ${escapeHtml(p.unit_path)}</span>`
      : '';
    const titleHint = state.sapViewEnabled && info.header_info && info.header_info.title_path === p.name
      ? ' <span class="text-ox-amber text-[10px]" title="Used as UI.HeaderInfo.Title">title</span>'
      : '';
    const semKeyHint = state.sapViewEnabled && Array.isArray(info.semantic_keys) && info.semantic_keys.includes(p.name)
      ? ' <span class="text-ox-amber text-[10px]" title="Common.SemanticKey — business-key property (vs the technical primary key)">biz key</span>'
      : '';
    const flagHints = state.sapViewEnabled ? propertyFlagHints(p) : '';
    const critHint = state.sapViewEnabled ? criticalityHint(p) : '';
    const vlHint = valueListHint(p, state.sapViewEnabled);
    // Dim the row when SAP View is on and Fiori would hide this
    // property (UI.Hidden or FieldControl=Hidden). Row stays visible
    // and clickable — the muted text just makes it visually recede.
    const hiddenRow = state.sapViewEnabled && (p.hidden || (p.field_control && p.field_control.kind === 'hidden'));
    const rowCls = hiddenRow ? 'opacity-60' : '';
    const nameCls = hiddenRow ? 'text-ox-dim' : 'text-ox-text';
    html += `<tr class="hover:bg-ox-amberGlow cursor-pointer transition-colors ${rowCls}" data-action="select" data-field="${escapeHtml(p.name)}">`;
    html += `<td class="py-0.5 pr-3 ${nameCls}">${escapeHtml(p.name)}${textHint}${currencyHint}${unitHint}${titleHint}${semKeyHint}${critHint}${flagHints}${vlHint}</td>`;
    html += `<td class="py-0.5 pr-3 text-ox-dim">${escapeHtml(p.edm_type.replace('Edm.', ''))}</td>`;
    html += `<td class="py-0.5 pr-3 text-center">${keyMark}</td>`;
    html += `<td class="py-0.5 text-ox-muted">${escapeHtml(p.label || '')}</td>`;
    html += '</tr>';
  }
  html += '</tbody></table></div>';

  // Nav properties
  if (info.nav_properties.length > 0) {
    html += '<div class="overflow-auto"><table class="w-full text-xs font-mono"><thead><tr class="text-ox-dim">';
    html += '<th class="text-left pb-1.5 bg-ox-surface pr-3">Navigation</th>';
    html += '<th class="text-left pb-1.5 bg-ox-surface pr-3">Target</th>';
    html += '<th class="text-left pb-1.5 bg-ox-surface">Mult.</th>';
    html += '</tr></thead><tbody>';
    for (const n of info.nav_properties) {
      html += `<tr class="hover:bg-ox-amberGlow cursor-pointer transition-colors" data-action="expand" data-field="${escapeHtml(n.name)}">`;
      html += `<td class="py-0.5 pr-3 text-ox-text">${escapeHtml(n.name)}</td>`;
      html += `<td class="py-0.5 pr-3 text-ox-dim">${escapeHtml(n.target_type)}</td>`;
      html += `<td class="py-0.5 text-ox-muted">${escapeHtml(n.multiplicity)}</td>`;
      html += '</tr>';
    }
    html += '</tbody></table></div>';
  }

  html += '</div>';
  if (state.sapViewEnabled) {
    html += renderFioriReadinessPanel(info);
  }
  document.getElementById('describeContent').innerHTML = html;
}

function hideDescribe() {
  document.getElementById('describePanel').classList.add('hidden');
}

// ══════════════════════════════════════════════════════════════
// ── CLICK-TO-ADD HELPERS ──
// ══════════════════════════════════════════════════════════════

function addToSelect(fieldName) {
  const el = document.getElementById('qSelect');
  const current = el.value.split(',').map(s => s.trim()).filter(Boolean);
  if (!current.includes(fieldName)) {
    current.push(fieldName);
    el.value = current.join(',');
  }
}

function addToExpand(navName) {
  const el = document.getElementById('qExpand');
  const current = el.value.split(',').map(s => s.trim()).filter(Boolean);
  if (!current.includes(navName)) {
    current.push(navName);
    el.value = current.join(',');
  }
}

// ══════════════════════════════════════════════════════════════
// ── ODATA URL BUILDER (for Copy URL feature) ──
// ══════════════════════════════════════════════════════════════

function buildODataUrl(params) {
  if (!state.currentServicePath || !params) return '';
  const parts = [];
  if (params.select)  parts.push(`$select=${encodeURIComponent(params.select)}`);
  if (params.filter)  parts.push(`$filter=${encodeURIComponent(params.filter)}`);
  if (params.expand)  parts.push(`$expand=${encodeURIComponent(params.expand)}`);
  if (params.orderby) parts.push(`$orderby=${encodeURIComponent(params.orderby)}`);
  if (params.top)     parts.push(`$top=${params.top}`);
  if (params.skip)    parts.push(`$skip=${params.skip}`);
  const qs = parts.length ? '?' + parts.join('&') : '';
  return `${state.currentServicePath}/${params.entity_set}${qs}`;
}

// ══════════════════════════════════════════════════════════════
// ── QUERY EXECUTION ──
// ══════════════════════════════════════════════════════════════

async function executeQuery(asJson = false) {
  if (!state.currentProfile || !state.currentServicePath || !state.currentEntitySet) {
    setStatus('Select a profile, service, and entity set first');
    return;
  }

  const params = {
    entity_set: state.currentEntitySet,
    select:  document.getElementById('qSelect').value  || null,
    filter:  document.getElementById('qFilter').value  || null,
    expand:  document.getElementById('qExpand').value  || null,
    orderby: document.getElementById('qOrderby').value || null,
    top:     parseInt(document.getElementById('qTop').value)  || null,
    skip:    parseInt(document.getElementById('qSkip').value) || null,
    key:     null,
    count:   false,
  };

  // SAP View pre-flight: when enabled, surface restriction warnings
  // without blocking. Annotations describe what Fiori would do; SAP
  // servers are often more permissive than the metadata claims, so the
  // server's response is the final word. We just make the context
  // visible before hitting send.
  if (state.sapViewEnabled) {
    const tab = getActiveTab();
    const info = tab && tab._lastDescribeInfo;
    if (info && info.name && state.currentEntitySet === document.getElementById('queryEntitySet').textContent) {
      showSapViewWarnings(validateQueryRestrictions(params, info));
    } else {
      showSapViewWarnings([]);
    }
  } else {
    showSapViewWarnings([]);
  }

  setStatus(`Querying ${state.currentEntitySet}...`);
  const queryStart = performance.now();
  const scope = tabScope();

  try {
    const data = await timedInvoke('run_query', {
      profileName: state.currentProfile,
      servicePath: state.currentServicePath,
      params,
    });
    if (!scope.active()) return;

    const elapsed = Math.round(performance.now() - queryStart);

    if (asJson) {
      renderJson(data);
      hideStatsBar();
    } else {
      renderResults(data, elapsed, params);
    }

    // Record in history
    const tab = getActiveTab();
    if (tab && !asJson) {
      const rows = extractRows(data);
      const rowCount = rows ? rows.length : 0;
      addToHistory(tab, params, rowCount, elapsed);
    }
  } catch (e) {
    if (!scope.active()) return;
    setStatus('Query error: ' + e);
    hideStatsBar();
    const message = isBrowserAuthProfile(state.currentProfile) ? browserAuthMessage(e) : String(e);
    document.getElementById('resultsArea').innerHTML =
      safeHtml`<div class="p-4 text-ox-red text-sm">${message}</div>`;
  }
}

// ══════════════════════════════════════════════════════════════
// ── STATS BAR (Feature 4) ──
// ══════════════════════════════════════════════════════════════

function showStatsBar(rowCount, sizeBytes, elapsedMs) {
  document.getElementById('statRows').textContent = `${rowCount} row${rowCount !== 1 ? 's' : ''}`;
  document.getElementById('statSize').textContent = formatBytes(sizeBytes);

  let timingClass = 'timing-fast';
  if (elapsedMs >= 2000) timingClass = 'timing-slow';
  else if (elapsedMs >= 500) timingClass = 'timing-ok';

  document.getElementById('statTiming').innerHTML =
    safeHtml`<span class="${timingClass}">${elapsedMs}ms</span>`;
  document.getElementById('statsBar').classList.remove('hidden');
}

function hideStatsBar() {
  document.getElementById('statsBar').classList.add('hidden');
}

function formatBytes(bytes) {
  if (bytes < 1024) return bytes + ' B';
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
  return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
}

// ══════════════════════════════════════════════════════════════
// ── QUERY HISTORY (Feature 3) ──
// ══════════════════════════════════════════════════════════════

function addToHistory(tab, params, rowCount, elapsed) {
  const entry = {
    ts: new Date(),
    entitySet: params.entity_set,
    params: { ...params },
    rowCount,
    elapsed,
    summary: buildParamSummary(params),
  };
  tab.queryHistory.unshift(entry);
  if (tab.queryHistory.length > 20) tab.queryHistory.length = 20;
  if (!document.getElementById('historyPanel').classList.contains('hidden')) {
    renderHistoryPanel(tab);
  }
}

function buildParamSummary(params) {
  const parts = [];
  if (params.select)  parts.push(`$select=${params.select}`);
  if (params.filter)  parts.push(`$filter=${params.filter}`);
  if (params.expand)  parts.push(`$expand=${params.expand}`);
  if (params.orderby) parts.push(`$orderby=${params.orderby}`);
  if (params.top)     parts.push(`$top=${params.top}`);
  if (params.skip)    parts.push(`$skip=${params.skip}`);
  return parts.join(' · ') || '(no params)';
}

function renderHistoryPanel(tab) {
  const panel = document.getElementById('historyPanel');
  if (!tab || tab.queryHistory.length === 0) {
    panel.innerHTML = '<div class="px-4 py-3 text-[11px] text-ox-dim font-mono">No history yet</div>';
    return;
  }
  let html = '<div class="flex items-center justify-between px-3 py-1 border-b border-ox-border">';
  html += '<span class="text-[9px] uppercase tracking-widest text-ox-dim font-medium">Query History</span>';
  html += '<button data-action="clear-history" class="text-[10px] text-ox-dim hover:text-ox-red px-1 transition-colors">clear</button>';
  html += '</div>';
  for (let i = 0; i < tab.queryHistory.length; i++) {
    const h = tab.queryHistory[i];
    const time = h.ts.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
    html += `<div class="history-item" data-action="replay-history" data-idx="${i}">
      <span class="text-ox-amber shrink-0">${escapeHtml(h.entitySet)}</span>
      <span class="text-ox-dim flex-1 truncate">${escapeHtml(h.summary)}</span>
      <span class="text-ox-dim shrink-0">${h.rowCount}r</span>
      <span class="text-ox-dim shrink-0">${h.elapsed}ms</span>
      <span class="text-ox-dim shrink-0">${time}</span>
    </div>`;
  }
  panel.innerHTML = html;
}

function replayHistory(idx) {
  const tab = getActiveTab();
  if (!tab) return;
  const h = tab.queryHistory[idx];
  if (!h) return;
  // Restore entity set and params into query bar
  if (h.entitySet) {
    state.currentEntitySet = h.entitySet;
    tab.entitySet = h.entitySet;
    document.getElementById('queryEntitySet').textContent = h.entitySet;
  }
  document.getElementById('qSelect').value  = h.params.select  || '';
  document.getElementById('qFilter').value  = h.params.filter  || '';
  document.getElementById('qExpand').value  = h.params.expand  || '';
  document.getElementById('qOrderby').value = h.params.orderby || '';
  document.getElementById('qTop').value     = h.params.top     || '20';
  document.getElementById('qSkip').value    = h.params.skip    || '';
  executeQuery(false);
}

// ══════════════════════════════════════════════════════════════
// ── HTTP INSPECTOR ──
// ══════════════════════════════════════════════════════════════

export function clearTraceState(tab = getActiveTab()) {
  if (!tab) return;
  tab.httpTraceEntries = [];
  tab.selectedTraceId = null;
  tab._traceVisible = false;
  if (tab.id === state.activeTabId) {
    document.getElementById('traceInspectorPanel').classList.add('hidden');
    renderTraceSummary(tab);
  }
}

export function ensureTraceSelection(tab) {
  if (!tab) return null;
  if (
    tab.selectedTraceId &&
    tab.httpTraceEntries.some(entry => entry.id === tab.selectedTraceId)
  ) {
    return tab.selectedTraceId;
  }
  tab.selectedTraceId = tab.httpTraceEntries.length
    ? tab.httpTraceEntries[tab.httpTraceEntries.length - 1].id
    : null;
  return tab.selectedTraceId;
}

function getSelectedTraceEntry(tab = getActiveTab()) {
  if (!tab) return null;
  const selectedId = ensureTraceSelection(tab);
  if (!selectedId) return null;
  return tab.httpTraceEntries.find(entry => entry.id === selectedId) || null;
}

export function renderTraceSummary(tab = getActiveTab()) {
  const count = tab?.httpTraceEntries?.length || 0;
  document.getElementById('traceSummary').textContent = count
    ? `${count} request${count === 1 ? '' : 's'}`
    : 'No trace';
  document.getElementById('traceCount').textContent = count
    ? `${count} request${count === 1 ? '' : 's'}`
    : 'No requests';
}

function traceStatusClass(entry) {
  if (entry.error) return 'err';
  if (!entry.status) return '';
  if (entry.status >= 500) return 'err';
  if (entry.status >= 400) return 'warn';
  if (entry.status >= 300) return 'warn';
  return 'ok';
}

function traceStatusLabel(entry) {
  if (entry.status) return String(entry.status);
  if (entry.error) return 'ERR';
  return 'OPEN';
}

function compactTraceUrl(url) {
  try {
    const parsed = new URL(url);
    return `${parsed.host}${parsed.pathname}${parsed.search}`;
  } catch {
    return url;
  }
}

function traceOutcomeLabel(entry) {
  if (entry.error) return entry.error;
  if (entry.redirect_location) return `redirect → ${entry.redirect_location}`;
  return entry.response_body_preview ? 'response captured' : 'headers captured';
}

function renderTraceHeaders(headers) {
  if (!headers || headers.length === 0) {
    return '<div class="trace-code text-ox-dim">No headers captured.</div>';
  }

  let html = '<div class="trace-header-grid">';
  for (const header of headers) {
    html += `<div class="trace-header-name">${escapeHtml(header.name)}</div>`;
    html += `<div class="trace-header-value">${escapeHtml(header.value)}</div>`;
  }
  html += '</div>';
  return html;
}

// First-render length for trace bodies — keeps the inspector snappy on
// large responses. The "show full" button below the preview reveals the
// rest (up to the core's MAX_BODY_PREVIEW_CHARS cap — anything beyond
// that arrives with a `... <truncated>` suffix already in place).
const TRACE_BODY_PREVIEW_CHARS = 4000;

function renderTraceBody(body, emptyLabel) {
  if (!body) {
    return `<div class="trace-code text-ox-dim">${escapeHtml(emptyLabel)}</div>`;
  }
  if (body.length <= TRACE_BODY_PREVIEW_CHARS) {
    return `<pre class="trace-code">${escapeHtml(body)}</pre>`;
  }
  // Long body — render collapsed, let the user opt in to full render.
  // `_traceBodyExpanded` is set per-tab when the expand button is clicked.
  const tab = getActiveTab();
  const expanded = tab && tab._traceBodyExpanded === true;
  const shown = expanded ? body : body.slice(0, TRACE_BODY_PREVIEW_CHARS) + '\n…';
  const sizeKb = (body.length / 1024).toFixed(1);
  const label = expanded
    ? `collapse (showing full ${sizeKb} KB)`
    : `show full response body (${sizeKb} KB)`;
  return `<pre class="trace-code">${escapeHtml(shown)}</pre>` +
    `<button type="button" class="mt-2 text-[10px] font-semibold tracking-wide px-2 py-1 rounded-sm text-ox-electric border border-ox-electric/50 hover:bg-ox-electric/10 hover:border-ox-electric transition-colors" data-action="toggle-trace-body">${escapeHtml(label)}</button>`;
}

function renderTraceList(tab) {
  const list = document.getElementById('traceList');
  if (!tab || tab.httpTraceEntries.length === 0) {
    list.innerHTML = '<div class="px-4 py-3 text-[11px] text-ox-dim font-mono">No traced requests yet.</div>';
    return;
  }

  const selectedId = ensureTraceSelection(tab);
  let html = '';
  for (const entry of [...tab.httpTraceEntries].reverse()) {
    const active = entry.id === selectedId ? ' active' : '';
    const statusClass = traceStatusClass(entry);
    const statusCls = statusClass ? ` ${statusClass}` : '';
    html += `<div class="trace-row${active}" data-action="select-trace" data-trace-id="${entry.id}">`;
    html += '<div class="trace-meta">';
    html += `<span class="trace-pill">${escapeHtml(entry.method)}</span>`;
    html += `<span class="trace-pill${statusCls}">${escapeHtml(traceStatusLabel(entry))}</span>`;
    html += `<span>${entry.duration_ms}ms</span>`;
    html += '</div>';
    html += `<div class="trace-url">${escapeHtml(compactTraceUrl(entry.url))}</div>`;
    html += `<div class="trace-meta">${escapeHtml(traceOutcomeLabel(entry))}</div>`;
    html += '</div>';
  }
  list.innerHTML = html;
}

function renderTraceDetail(tab) {
  const detail = document.getElementById('traceDetail');
  const entry = getSelectedTraceEntry(tab);
  if (!entry) {
    detail.innerHTML = '<div class="px-4 py-4 text-[11px] text-ox-dim font-mono">Select a traced request to inspect it.</div>';
    return;
  }

  const activeSubTab = tab?._traceSubTab === 'request' ? 'request' : 'response';
  const statusClass = traceStatusClass(entry);
  const statusCls = statusClass ? ` ${statusClass}` : '';

  let html = '<div class="trace-section">';
  html += '<div class="flex items-center gap-2 mb-2">';
  html += `<span class="trace-pill">${escapeHtml(entry.method)}</span>`;
  html += `<span class="trace-pill${statusCls}">${escapeHtml(traceStatusLabel(entry))}</span>`;
  html += `<span class="trace-meta">${entry.duration_ms}ms</span>`;
  html += '</div>';
  html += `<div class="trace-url">${escapeHtml(entry.url)}</div>`;
  html += '</div>';

  html += '<div class="trace-subtabs">';
  html += `<div class="trace-subtab${activeSubTab === 'request' ? ' active' : ''}" data-action="select-trace-subtab" data-subtab="request">Request</div>`;
  html += `<div class="trace-subtab${activeSubTab === 'response' ? ' active' : ''}" data-action="select-trace-subtab" data-subtab="response">Response</div>`;
  html += '<div class="trace-subtab-actions">';
  if (activeSubTab === 'request') {
    html += '<button data-action="copy-trace-curl">copy as curl</button>';
    const disabled = entry.request_body_preview ? '' : ' disabled';
    html += `<button data-action="copy-trace-request-body"${disabled}>copy body</button>`;
  } else {
    const disabled = entry.response_body_preview ? '' : ' disabled';
    html += `<button data-action="copy-trace-response-body"${disabled}>copy body</button>`;
  }
  html += '</div>';
  html += '</div>';

  if (activeSubTab === 'request') {
    html += '<div class="trace-section">';
    html += '<div class="trace-section-title">Headers</div>';
    html += renderTraceHeaders(entry.request_headers);
    html += '</div>';

    html += '<div class="trace-section">';
    html += '<div class="trace-section-title">Body</div>';
    html += renderTraceBody(entry.request_body_preview, 'No request body captured.');
    html += '</div>';
  } else {
    html += '<div class="trace-section">';
    html += '<div class="trace-section-title">Headers</div>';
    html += renderTraceHeaders(entry.response_headers);
    html += '</div>';

    html += '<div class="trace-section">';
    html += '<div class="trace-section-title">Body Preview</div>';
    html += renderTraceBody(entry.response_body_preview, 'No response body preview captured.');
    html += '</div>';

    if (entry.redirect_location) {
      html += '<div class="trace-section">';
      html += '<div class="trace-section-title">Redirect</div>';
      html += `<div class="trace-code">${escapeHtml(entry.redirect_location)}</div>`;
      html += '</div>';
    }

    if (entry.error) {
      html += '<div class="trace-section">';
      html += '<div class="trace-section-title">Error</div>';
      html += `<pre class="trace-code">${escapeHtml(entry.error)}</pre>`;
      html += '</div>';
    }
  }

  detail.innerHTML = html;
}

export function renderTraceInspector(tab = getActiveTab()) {
  renderTraceSummary(tab);
  renderTraceList(tab);
  renderTraceDetail(tab);
}

function showTraceInspector() {
  const tab = getActiveTab();
  if (!tab) return;
  tab._traceVisible = true;
  renderTraceInspector(tab);
  document.getElementById('traceInspectorPanel').classList.remove('hidden');
  updateTraceToggleState(true);
}

function hideTraceInspector() {
  const tab = getActiveTab();
  if (tab) tab._traceVisible = false;
  document.getElementById('traceInspectorPanel').classList.add('hidden');
  updateTraceToggleState(false);
}

function updateTraceToggleState(open) {
  const btn = document.getElementById('btnTraceToggle');
  const chevron = document.getElementById('traceToggleChevron');
  if (!btn || !chevron) return;
  chevron.innerHTML = open ? '&#x25BE;' : '&#x25B4;';
  // Off = dim green (primed / telemetry standby). On = full green
  // with a subtle glow. Distinct from SAP View's amber so the two
  // status-bar toggles read as different kinds of switch at a glance.
  if (open) {
    btn.classList.add('text-ox-green', 'border-ox-green', 'bg-ox-greenGlow');
    btn.classList.remove('text-ox-greenDim', 'border-ox-greenDim/60');
  } else {
    btn.classList.add('text-ox-greenDim', 'border-ox-greenDim/60');
    btn.classList.remove('text-ox-green', 'border-ox-green', 'bg-ox-greenGlow');
  }
}

function updateSapViewToggleState() {
  const btn = document.getElementById('btnSapView');
  const chev = document.getElementById('sapViewChevron');
  if (!btn || !chev) return;
  chev.innerHTML = state.sapViewEnabled ? '&#x25BE;' : '&#x25B4;';
  // Off state = dim amber (primed but inactive). On state = full amber
  // with a subtle glow so it reads clearly as "engaged".
  if (state.sapViewEnabled) {
    btn.classList.add('text-ox-amber', 'border-ox-amber', 'bg-ox-amberGlow');
    btn.classList.remove('text-ox-amberDim', 'border-ox-amberDim/60');
  } else {
    btn.classList.add('text-ox-amberDim', 'border-ox-amberDim/60');
    btn.classList.remove('text-ox-amber', 'border-ox-amber', 'bg-ox-amberGlow');
  }
}

function toggleSapView() {
  state.sapViewEnabled = !state.sapViewEnabled;
  try { localStorage.setItem('ox_sap_view_enabled', state.sapViewEnabled ? '1' : '0'); } catch { /* ignore */ }
  updateSapViewToggleState();
  // Re-render describe panel in place if the active tab has one up.
  // renderDescribe also refreshes the selection-fields chip bar.
  const tab = getActiveTab();
  if (tab && tab._lastDescribeInfo) {
    renderDescribe(tab._lastDescribeInfo);
  } else {
    // No cached describe — still hide any stale chip bar and quick-actions.
    renderSelectionFieldsBar(null);
    renderFioriColsButton(null);
    renderFioriFilterButton(null);
  }
  // Clear any lingering warnings from the previous mode.
  if (!state.sapViewEnabled) showSapViewWarnings([]);
}

function toggleTraceInspector() {
  const panel = document.getElementById('traceInspectorPanel');
  if (panel.classList.contains('hidden')) {
    showTraceInspector();
  } else {
    hideTraceInspector();
  }
}

// POSIX-shell single-quote escape. The resulting curl command runs in bash /
// zsh / git-bash, but cmd.exe and PowerShell use different quoting rules —
// paste into those shells and the quotes will leak through literally.
function shellQuoteForCurl(value) {
  return "'" + String(value).replace(/'/g, `'\"'\"'`) + "'";
}

// ══════════════════════════════════════════════════════════════
// ── ANNOTATION BADGE (footer) ──
// ══════════════════════════════════════════════════════════════
// Small status-bar badge showing the raw annotation count for the
// currently loaded service, with a hover breakdown by vocabulary
// namespace. The thin slice — the typed feature views (criticality,
// Text arrangement, Fiori-readiness, etc.) will layer on top.

export function renderAnnotationBadge(summary) {
  const el = document.getElementById('annotationBadge');
  if (!el) return;
  const total = summary && typeof summary.total === 'number' ? summary.total : 0;
  if (total === 0) {
    el.classList.add('hidden');
    el.textContent = '';
    el.title = '';
    return;
  }
  el.classList.remove('hidden');
  el.textContent = `${total} annotation${total === 1 ? '' : 's'}`;
  const byNs = summary.by_namespace || {};
  const lines = Object.entries(byNs)
    .sort(([, a], [, b]) => b - a)
    .map(([ns, count]) => `${ns}: ${count}`);
  el.title = lines.length
    ? `${lines.join('\n')}\n\nClick to open the annotation inspector`
    : `${total} annotations — click to inspect`;
}

// ══════════════════════════════════════════════════════════════
// ── ANNOTATION INSPECTOR (modal) ──
// ══════════════════════════════════════════════════════════════
// Lazy-loaded dump of every raw annotation the parser captured. Good
// for answering "does this service declare X?" when the feature view
// doesn't surface it yet, or for grepping across namespaces. Cached
// per service path so reopening is instant.
let _aiAnnotations = [];
let _aiActiveNamespaces = new Set(); // empty = show all
const _aiCache = new Map();

async function openAnnotationInspector() {
  const modal = document.getElementById('annotationInspectorModal');
  const subtitle = document.getElementById('aiSubtitle');
  const search = document.getElementById('aiSearch');
  const results = document.getElementById('aiResults');
  if (!modal) return;
  if (!state.currentProfile || !state.currentServicePath) return;
  modal.classList.remove('hidden');
  subtitle.textContent = state.currentServicePath;
  search.value = '';
  _aiActiveNamespaces = new Set();
  const cacheKey = `${state.currentProfile}::${state.currentServicePath}`;
  let annotations = _aiCache.get(cacheKey);
  if (!annotations) {
    results.innerHTML = '<div class="p-4 text-ox-dim text-[11px]">Loading annotations…</div>';
    try {
      annotations = await timedInvoke('get_annotations', {
        profileName: state.currentProfile,
        servicePath: state.currentServicePath,
      });
      _aiCache.set(cacheKey, annotations);
    } catch (e) {
      results.innerHTML = safeHtml`<div class="p-4 text-ox-red text-[11px]">Could not load annotations:\n${String(e)}</div>`;
      return;
    }
  }
  _aiAnnotations = Array.isArray(annotations) ? annotations : [];
  renderAnnotationInspector();
  setTimeout(() => search.focus(), 0);
}

function closeAnnotationInspector() {
  document.getElementById('annotationInspectorModal').classList.add('hidden');
}

// Re-render the inspector's filtered table + namespace chips against
// the currently-loaded annotation list, the text filter, and the
// active namespace toggles.
function renderAnnotationInspector() {
  const results = document.getElementById('aiResults');
  const nsBar = document.getElementById('aiNamespaceBar');
  const countEl = document.getElementById('aiCount');
  const searchEl = document.getElementById('aiSearch');
  const needle = (searchEl.value || '').trim().toLowerCase();
  // Build namespace list + counts from the FULL annotation set so the
  // chip bar doesn't flicker when filters toggle.
  const nsCounts = new Map();
  for (const a of _aiAnnotations) {
    nsCounts.set(a.namespace, (nsCounts.get(a.namespace) || 0) + 1);
  }
  const sortedNs = [...nsCounts.entries()].sort(([, a], [, b]) => b - a);
  nsBar.innerHTML = sortedNs
    .map(([ns, count]) => {
      const on = _aiActiveNamespaces.size === 0 || _aiActiveNamespaces.has(ns);
      const base = 'text-[10px] font-semibold tracking-wide px-1.5 py-0.5 rounded-sm transition-colors cursor-pointer';
      const onCls = 'text-ox-blue border border-ox-blue bg-ox-blue/10';
      const offCls = 'text-ox-dim border border-ox-border hover:text-ox-blue hover:border-ox-blue/50';
      const cls = on ? `${base} ${onCls}` : `${base} ${offCls}`;
      return `<button type="button" class="${cls}" data-action="ai-toggle-ns" data-ns="${escapeHtml(ns)}">${escapeHtml(ns)} · ${count}</button>`;
    })
    .join('');
  // Filter by namespace + text search.
  const filtered = _aiAnnotations.filter(a => {
    if (_aiActiveNamespaces.size > 0 && !_aiActiveNamespaces.has(a.namespace)) return false;
    if (needle) {
      const hay = [a.term, a.target, a.value || '', a.qualifier || '']
        .join(' ').toLowerCase();
      if (!hay.includes(needle)) return false;
    }
    return true;
  });
  countEl.textContent = `${filtered.length} of ${_aiAnnotations.length}`;
  if (filtered.length === 0) {
    results.innerHTML = '<div class="p-4 text-ox-dim text-[11px]">No matching annotations.</div>';
    return;
  }
  let html = '<table class="w-full border-collapse">';
  html += '<thead class="sticky top-0 bg-ox-surface z-10"><tr class="text-ox-dim text-[10px]">';
  html += '<th class="text-left px-3 py-1 border-b border-ox-border w-[110px]">Namespace</th>';
  html += '<th class="text-left px-3 py-1 border-b border-ox-border">Term</th>';
  html += '<th class="text-left px-3 py-1 border-b border-ox-border">Target</th>';
  html += '<th class="text-left px-3 py-1 border-b border-ox-border">Value</th>';
  html += '<th class="text-left px-3 py-1 border-b border-ox-border w-[90px]">Qualifier</th>';
  html += '</tr></thead><tbody>';
  for (const a of filtered) {
    const value = a.value === null || a.value === undefined ? '' : String(a.value);
    const qualifier = a.qualifier || '';
    html += '<tr class="border-b border-ox-border/30 hover:bg-ox-hover/40">';
    html += `<td class="px-3 py-0.5 text-ox-blue">${escapeHtml(a.namespace)}</td>`;
    html += `<td class="px-3 py-0.5 text-ox-text">${escapeHtml(a.term)}</td>`;
    html += `<td class="px-3 py-0.5 text-ox-muted">${escapeHtml(a.target)}</td>`;
    html += `<td class="px-3 py-0.5 text-ox-text">${escapeHtml(value) || '<span class="text-ox-dim">—</span>'}</td>`;
    html += `<td class="px-3 py-0.5 text-ox-muted">${escapeHtml(qualifier)}</td>`;
    html += '</tr>';
  }
  html += '</tbody></table>';
  results.innerHTML = html;
}

function traceToCurl(entry) {
  const parts = [
    `curl -X ${shellQuoteForCurl(entry.method)}`,
    `--url ${shellQuoteForCurl(entry.url)}`,
  ];
  for (const header of entry.request_headers || []) {
    parts.push(`-H ${shellQuoteForCurl(`${header.name}: ${header.value}`)}`);
  }
  if (entry.request_body_preview) {
    parts.push(`--data-raw ${shellQuoteForCurl(entry.request_body_preview)}`);
  }
  return parts.join(' ');
}

async function copySelectedTraceAsCurl() {
  const entry = getSelectedTraceEntry(getActiveTab());
  if (!entry) {
    setStatus('No trace selected');
    return;
  }
  await copyToClipboard(traceToCurl(entry), 'curl command');
}

async function copySelectedTraceRequestBody() {
  const entry = getSelectedTraceEntry(getActiveTab());
  if (!entry || !entry.request_body_preview) {
    setStatus('No request body to copy');
    return;
  }
  await copyToClipboard(entry.request_body_preview, 'request body');
}

async function copySelectedTraceResponseBody() {
  const entry = getSelectedTraceEntry(getActiveTab());
  if (!entry || !entry.response_body_preview) {
    setStatus('No response body to copy');
    return;
  }
  await copyToClipboard(entry.response_body_preview, 'response body');
}

// ══════════════════════════════════════════════════════════════
// ── RESULTS RENDERING ──
// ══════════════════════════════════════════════════════════════

function extractRows(data) {
  if (data.d) {
    if (data.d.results) return data.d.results;
    return [data.d];
  }
  if (data.value) return data.value;
  return null;
}

function renderResults(data, elapsedMs, params) {
  const rows = extractRows(data);
  if (!rows || rows.length === 0) {
    document.getElementById('resultsArea').innerHTML =
      '<div class="p-4 text-ox-dim text-sm">No results</div>';
    setStatus('No results');
    hideStatsBar();
    return;
  }

  state.expandedDataStore = {};
  state.lastResultRows = rows;
  const first = rows[0];

  const scalarCols = [];
  const nestedCols = [];
  for (const k of Object.keys(first)) {
    if (k.startsWith('@') || k === '__metadata') continue;
    const val = first[k];
    if (val !== null && typeof val === 'object') {
      nestedCols.push(k);
    } else {
      scalarCols.push(k);
    }
  }

  // SAP-View-driven reshaping: column order from UI.LineItem, plus a
  // propByName lookup used below to render cells per UI.TextArrangement
  // and to hide text-companion columns when they're already folded
  // into their ID column.
  const tab = getActiveTab();
  const info = tab && tab._lastDescribeInfo;
  const sapShape = state.sapViewEnabled && info && Array.isArray(info.properties);
  const propByName = sapShape ? new Map(info.properties.map(p => [p.name, p])) : null;
  // Text-companion columns that will be folded into their ID column's
  // cell — we skip rendering them as standalone columns when the
  // arrangement is anything other than TextSeparate.
  const foldedTextCols = new Set();
  if (sapShape) {
    for (const p of info.properties) {
      if (!p.text_path) continue;
      const arrangement = p.text_arrangement || 'textfirst';
      if (arrangement !== 'textseparate') foldedTextCols.add(p.text_path);
    }
  }
  // Reorder scalars: declared LineItem columns first in position order,
  // then whatever else the response carried (keys, RequestAtLeast
  // fields, technical columns). Only applies in SAP View.
  let orderedScalars = scalarCols;
  if (sapShape && Array.isArray(info.line_item) && info.line_item.length > 0) {
    const scalarSet = new Set(scalarCols);
    const seen = new Set();
    const head = [];
    for (const li of info.line_item) {
      const name = li && li.value_path;
      if (name && scalarSet.has(name) && !seen.has(name)) {
        head.push(name);
        seen.add(name);
      }
    }
    const tail = scalarCols.filter(c => !seen.has(c));
    orderedScalars = [...head, ...tail];
  }
  if (sapShape) {
    orderedScalars = orderedScalars.filter(c => !foldedTextCols.has(c));
  }

  const allCols = [...orderedScalars, ...nestedCols];

  // Estimate JSON size for stats
  const jsonSize = new Blob([JSON.stringify(data)]).size;
  showStatsBar(rows.length, jsonSize, elapsedMs || 0);

  let html = '<div class="overflow-auto h-full">';
  html += '<table class="w-full text-xs font-mono border-collapse">';
  html += '<thead><tr>';
  for (const col of allCols) {
    const isNested = nestedCols.includes(col);
    const label = isNested ? `${col} ↗` : col;
    html += `<th class="text-left px-3 py-1.5 bg-ox-panel text-ox-dim border-b border-ox-border font-medium sticky top-0 group">`;
    html += `<span class="mr-1">${escapeHtml(label)}</span>`;
    if (!isNested) {
      // Copy column button (Feature 5)
      html += `<button class="copy-btn" data-action="copy-col" data-col="${escapeHtml(col)}" title="Copy column values">`;
      html += `<svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>`;
      html += `</button>`;
    }
    html += `</th>`;
  }
  // Row copy column header
  html += `<th class="text-left px-2 py-1.5 bg-ox-panel border-b border-ox-border sticky top-0 w-6"></th>`;
  html += '</tr></thead><tbody>';

  for (let i = 0; i < rows.length; i++) {
    const row = rows[i];
    const stripe = i % 2 === 0 ? '' : 'bg-ox-surface/50';
    html += `<tr class="hover:bg-ox-amberGlow border-b border-ox-border/30 transition-colors ${stripe}" data-row-idx="${i}">`;

    for (const col of allCols) {
      const val = row[col];
      if (val === null || val === undefined) {
        html += `<td class="px-3 py-1 text-ox-dim">—</td>`;
      } else if (Array.isArray(val)) {
        const storeKey = `r${i}_${col}`;
        state.expandedDataStore[storeKey] = val;
        const count = val.length;
        html += `<td class="px-3 py-1"><span class="expand-badge text-[10px] px-1.5 py-0.5 rounded-sm font-mono inline-block" data-action="nested" data-key="${storeKey}" data-col="${escapeHtml(col)}">${count} item${count !== 1 ? 's' : ''}</span></td>`;
      } else if (typeof val === 'object') {
        const storeKey = `r${i}_${col}`;
        state.expandedDataStore[storeKey] = val;
        html += `<td class="px-3 py-1"><span class="expand-badge text-[10px] px-1.5 py-0.5 rounded-sm font-mono inline-block" data-action="nested" data-key="${storeKey}" data-col="${escapeHtml(col)}">object</span></td>`;
      } else {
        const text = String(val);
        // SAP View: apply sap:display-format to the raw value first
        // (Date strips the time, UpperCase upcases, ...), THEN compose
        // with Common.Text per UI.TextArrangement. The raw `text` is
        // still what goes into filter-tooltip data attributes so
        // click-to-filter uses the unmodified key.
        const prop = sapShape ? propByName.get(col) : null;
        const formatted = prop && prop.display_format
          ? formatDisplayValue(text, prop.display_format, prop.edm_type)
          : text;
        let display = formatted;
        if (sapShape && prop && prop.text_path) {
          const companion = row[prop.text_path];
          const companionText = companion === null || companion === undefined
            ? ''
            : String(companion);
          const arr = prop.text_arrangement || 'textfirst';
          if (arr === 'textfirst' && companionText) {
            display = `${companionText} (${formatted})`;
          } else if (arr === 'textlast' && companionText) {
            display = `${formatted} (${companionText})`;
          } else if (arr === 'textonly') {
            display = companionText || formatted;
          }
        }
        const critDot = sapShape ? criticalityDot(prop, row) : '';
        html += `<td class="px-3 py-1 text-ox-text whitespace-nowrap cursor-pointer" data-action="cell-click" data-cell-col="${escapeHtml(col)}" data-cell-val="${escapeHtml(text)}">${critDot}${escapeHtml(display)}</td>`;
      }
    }

    // Row copy button (Feature 5)
    const storeKey = `row_${i}`;
    state.expandedDataStore[storeKey] = row;
    html += `<td class="px-2 py-1">`;
    html += `<button class="copy-btn row-copy-btn" data-action="copy-row" data-key="${storeKey}" title="Copy row as JSON">`;
    html += `<svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>`;
    html += `</button></td>`;
    html += '</tr>';
  }

  html += '</tbody></table></div>';
  document.getElementById('resultsArea').innerHTML = html;

  // state.lastResultRows already set above for copy operations

  setStatus(`${rows.length} row(s)${nestedCols.length ? ' — click badges to view expanded data' : ''}`);
}

function showNestedData(storeKey, colName) {
  const data = state.expandedDataStore[storeKey];
  if (!data) return;

  const rows = Array.isArray(data) ? data : [data];
  if (rows.length === 0) { alert('No nested data'); return; }

  const first = rows[0];
  if (typeof first !== 'object' || first === null) {
    const json = JSON.stringify(data, null, 2);
    showNestedPanel(colName, safeHtml`<pre class="text-xs font-mono text-ox-text p-3 whitespace-pre">${json}</pre>`);
    return;
  }

  const cols = Object.keys(first).filter(k => !k.startsWith('@') && k !== '__metadata');

  let html = '<div class="overflow-auto max-h-64"><table class="w-full text-xs font-mono border-collapse">';
  html += '<thead><tr>';
  for (const c of cols) {
    html += `<th class="text-left px-2 py-1 bg-ox-panel text-ox-dim border-b border-ox-border font-medium sticky top-0">${escapeHtml(c)}</th>`;
  }
  html += '</tr></thead><tbody>';
  for (const row of rows) {
    html += '<tr class="border-b border-ox-border/30">';
    for (const c of cols) {
      const v = row[c];
      const t = (v === null || v === undefined) ? '' : (typeof v === 'object' ? JSON.stringify(v) : String(v));
      html += `<td class="px-2 py-0.5 text-ox-text whitespace-nowrap">${escapeHtml(t)}</td>`;
    }
    html += '</tr>';
  }
  html += '</tbody></table></div>';
  showNestedPanel(colName, html);
}

function showNestedPanel(title, contentHtml) {
  const existing = document.getElementById('nestedPanel');
  if (existing) existing.remove();

  const panel = document.createElement('div');
  panel.id = 'nestedPanel';
  panel.className = 'fixed bottom-8 right-4 w-[600px] bg-ox-panel border border-ox-border rounded-lg shadow-2xl z-40';
  panel.style.animation = 'slideUp 0.2s ease';
  // contentHtml is built by callers (showNestedData, etc.) using safeHtml
  // for every interpolation, so we pass it through unescaped via raw().
  panel.innerHTML = safeHtml`
    <div class="px-4 py-2 border-b border-ox-border flex items-center justify-between">
      <div class="flex items-center gap-2">
        <div class="w-1.5 h-1.5 rounded-full bg-ox-blue"></div>
        <span class="text-xs font-mono font-medium text-ox-text">${title}</span>
      </div>
      <button data-action="close-nested" class="text-ox-dim hover:text-ox-text text-xs px-2 py-0.5 rounded-sm hover:bg-ox-hover">close</button>
    </div>
    <div class="p-2">${raw(contentHtml)}</div>
  `;
  document.body.appendChild(panel);
}

function renderJson(data) {
  const json = JSON.stringify(data, null, 2);
  document.getElementById('resultsArea').innerHTML =
    safeHtml`<pre class="text-xs font-mono text-ox-text p-4 overflow-auto h-full whitespace-pre leading-relaxed">${json}</pre>`;
  setStatus('JSON');
}

// ══════════════════════════════════════════════════════════════
// ── COPY FUNCTIONS (Feature 5) ──
// ══════════════════════════════════════════════════════════════

async function copyToClipboard(text, label) {
  try {
    await navigator.clipboard.writeText(text);
    setStatus(`Copied ${label || 'to clipboard'}`);
  } catch (e) {
    setStatus('Copy failed: ' + e);
  }
}

function copyColumnValues(colName) {
  const rows = state.lastResultRows || [];
  const values = rows.map(r => {
    const v = r[colName];
    return (v === null || v === undefined) ? '' : String(v);
  });
  copyToClipboard(values.join('\n'), `column "${colName}"`);
}

function copyRowAsJson(storeKey) {
  const row = state.expandedDataStore[storeKey];
  if (!row) return;
  const json = JSON.stringify(row, null, 2);
  copyToClipboard(json, 'row as JSON');
}

function copyODataUrl() {
  const params = {
    entity_set: state.currentEntitySet,
    select:  document.getElementById('qSelect').value  || null,
    filter:  document.getElementById('qFilter').value  || null,
    expand:  document.getElementById('qExpand').value  || null,
    orderby: document.getElementById('qOrderby').value || null,
    top:     parseInt(document.getElementById('qTop').value)  || null,
    skip:    parseInt(document.getElementById('qSkip').value) || null,
  };
  const url = buildODataUrl(params);
  if (url) copyToClipboard(url, 'OData URL');
  else setStatus('No service/entity selected');
}

// ══════════════════════════════════════════════════════════════
// ── FILTER TOOLTIP (Feature 7) ──
// ══════════════════════════════════════════════════════════════

let filterTooltipTimeout = null;

function showFilterTooltip(col, val, x, y) {
  clearTimeout(filterTooltipTimeout);
  const tt = document.getElementById('filterTooltip');
  const escapedVal = val.replace(/'/g, "''"); // OData escapes single quotes by doubling
  tt.textContent = `Filter: ${col} eq '${val}'`;
  tt.dataset.col = col;
  tt.dataset.val = escapedVal;
  tt.style.left = `${x}px`;
  tt.style.top  = `${y + 8}px`;
  tt.style.display = 'block';

  filterTooltipTimeout = setTimeout(hideFilterTooltip, 4000);
}

function hideFilterTooltip() {
  clearTimeout(filterTooltipTimeout);
  document.getElementById('filterTooltip').style.display = 'none';
}

function applyFilterFromTooltip() {
  const tt = document.getElementById('filterTooltip');
  const col = tt.dataset.col;
  const val = tt.dataset.val;
  if (!col) return;
  const filterVal = `${col} eq '${val}'`;
  document.getElementById('qFilter').value = filterVal;
  hideFilterTooltip();
  // Auto-run
  executeQuery(false);
}

// ══════════════════════════════════════════════════════════════
// ── ADD PROFILE MODAL ──
// ══════════════════════════════════════════════════════════════

function showAddProfileModal() {
  document.getElementById('addProfileModal').classList.remove('hidden');
  document.getElementById('mpName').value = '';
  document.getElementById('mpUrl').value = '';
  document.getElementById('mpClient').value = '100';
  document.getElementById('mpLang').value = 'EN';
  document.getElementById('mpAuthMode').value = 'basic';
  document.getElementById('mpUser').value = '';
  document.getElementById('mpPass').value = '';
  updateAuthModeFields();
  document.getElementById('mpError').classList.add('hidden');
  document.getElementById('mpSuccess').classList.add('hidden');
  document.getElementById('mpName').focus();
}

function updateAuthModeFields() {
  const mode = document.getElementById('mpAuthMode').value;
  document.getElementById('mpCredFields').style.display = mode === 'basic' ? '' : 'none';

  const hint = document.getElementById('mpAuthHint');
  if (mode === 'sso') {
    hint.textContent = 'Uses Windows integrated auth via Kerberos / Negotiate.';
  } else if (mode === 'browser') {
    hint.textContent = 'Opens an in-app sign-in window for Azure AD / SAP IAS style browser authentication.';
  } else {
    hint.textContent = 'Stores the password in Windows Credential Manager.';
  }
}

function hideAddProfileModal() {
  document.getElementById('addProfileModal').classList.add('hidden');
}

async function saveProfileModal() {
  const name     = document.getElementById('mpName').value.trim();
  const url      = document.getElementById('mpUrl').value.trim();
  const client   = document.getElementById('mpClient').value.trim();
  const language = document.getElementById('mpLang').value.trim();
  const authMode = document.getElementById('mpAuthMode').value;
  const user     = authMode === 'basic' ? document.getElementById('mpUser').value.trim() : '';
  const pass     = authMode === 'basic' ? document.getElementById('mpPass').value : '';

  const errEl = document.getElementById('mpError');
  const okEl  = document.getElementById('mpSuccess');
  errEl.classList.add('hidden');
  okEl.classList.add('hidden');

  if (!name || !url) {
    errEl.textContent = 'Profile name and URL are required';
    errEl.classList.remove('hidden');
    return;
  }
  if (authMode === 'basic' && (!user || !pass)) {
    errEl.textContent = 'Username and password are required for basic authentication';
    errEl.classList.remove('hidden');
    return;
  }

  const doSave = async (allowPlaintextFallback) => {
    return await invoke('add_profile', {
      name, baseUrl: url, client, language, authMode, username: user, password: pass,
      allowPlaintextFallback,
    });
  };

  try {
    let msg;
    try {
      msg = await doSave(false);
    } catch (e) {
      const errStr = String(e);
      // Backend signals keyring failure with a specific prefix so we can offer
      // an explicit confirmation instead of silently downgrading to plaintext.
      if (errStr.includes('KEYRING_FAILED')) {
        const proceed = window.confirm(
          'The OS keyring is unavailable or rejected the password.\n\n' +
          'Store the password in the config file as plaintext instead?\n' +
          '(Not recommended — the file is only protected by your OS file permissions.)'
        );
        if (!proceed) throw e;
        msg = await doSave(true);
      } else {
        throw e;
      }
    }
    okEl.textContent = msg;
    okEl.classList.remove('hidden');
    await loadProfiles();
    document.getElementById('profileSelect').value = name;
    document.getElementById('profileSelect').dispatchEvent(new Event('change'));
    setTimeout(hideAddProfileModal, 800);
  } catch (e) {
    errEl.textContent = String(e).replace(/^KEYRING_FAILED:\s*/, '');
    errEl.classList.remove('hidden');
  }
}

async function testProfileModal() {
  const url    = document.getElementById('mpUrl').value.trim();
  const client = document.getElementById('mpClient').value.trim();
  const language = document.getElementById('mpLang').value.trim() || 'EN';
  const authMode = document.getElementById('mpAuthMode').value;
  const user   = authMode === 'basic' ? document.getElementById('mpUser').value.trim() : '';
  const pass   = authMode === 'basic' ? document.getElementById('mpPass').value : '';
  const name   = document.getElementById('mpName').value.trim();

  const errEl = document.getElementById('mpError');
  const okEl  = document.getElementById('mpSuccess');
  errEl.classList.add('hidden');
  okEl.classList.add('hidden');

  if (!name || !url) {
    errEl.textContent = 'Fill in name and URL first';
    errEl.classList.remove('hidden');
    return;
  }

  try {
    const msg = await timedInvoke('test_connection', {
      baseUrl: url, client, language, authMode, username: user, password: pass,
    });
    okEl.textContent = msg;
    okEl.classList.remove('hidden');
  } catch (e) {
    errEl.textContent = String(e);
    errEl.classList.remove('hidden');
  }
}

// ══════════════════════════════════════════════════════════════
// ── KEYBOARD SHORTCUTS (Feature 8) ──
// ══════════════════════════════════════════════════════════════

document.addEventListener('keydown', (e) => {
  // Escape — close modals / panels
  if (e.key === 'Escape') {
    if (!document.getElementById('addProfileModal').classList.contains('hidden')) {
      hideAddProfileModal();
      return;
    }
    const nested = document.getElementById('nestedPanel');
    if (nested) { nested.remove(); return; }
    hideFilterTooltip();
    return;
  }

  // Ctrl+Enter — run query
  if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
    const active = document.activeElement;
    // Only if focus is somewhere in the query bar or results area
    const inQueryZone =
      active && (
        active.id === 'qSelect' ||
        active.id === 'qFilter' ||
        active.id === 'qExpand' ||
        active.id === 'qOrderby' ||
        active.id === 'qTop' ||
        active.id === 'qSkip'
      );
    if (inQueryZone || !document.getElementById('queryBar').classList.contains('hidden')) {
      e.preventDefault();
      executeQuery(false);
    }
    return;
  }

  // Enter in service input → search
  if (e.key === 'Enter' && document.activeElement === document.getElementById('serviceInput')) {
    loadService();
  }
});

// ══════════════════════════════════════════════════════════════
// ── INIT ──
// ══════════════════════════════════════════════════════════════

document.addEventListener('DOMContentLoaded', () => {
  // Create first tab
  addTab({ title: 'New Tab' });

  loadProfiles();

  // ── Static button wiring ──
  document.getElementById('btnAddProfile').addEventListener('click', showAddProfileModal);
  document.getElementById('btnProfileSignIn').addEventListener('click', signInCurrentProfile);
  document.getElementById('btnProfileSignOut').addEventListener('click', signOutCurrentProfile);
  document.getElementById('btnRemoveProfile').addEventListener('click', removeCurrentProfile);
  document.getElementById('btnSearch').addEventListener('click', loadService);
  document.getElementById('btnCloseDescribe').addEventListener('click', hideDescribe);
  document.getElementById('btnRun').addEventListener('click', () => executeQuery(false));
  document.getElementById('btnJson').addEventListener('click', () => executeQuery(true));
  document.getElementById('btnCopyUrl').addEventListener('click', copyODataUrl);
  document.getElementById('btnTraceToggle').addEventListener('click', toggleTraceInspector);
  document.getElementById('btnTraceClose').addEventListener('click', hideTraceInspector);
  document.getElementById('btnSapView').addEventListener('click', toggleSapView);
  document.getElementById('annotationBadge').addEventListener('click', openAnnotationInspector);
  document.getElementById('annotationBadge').classList.add('cursor-pointer');
  document.getElementById('btnAiClose').addEventListener('click', closeAnnotationInspector);
  document.getElementById('aiSearch').addEventListener('input', renderAnnotationInspector);
  document.getElementById('btnOpenFilterBar').addEventListener('click', openFilterBar);
  document.getElementById('btnFbClose').addEventListener('click', closeFilterBar);
  document.getElementById('btnFbCancel').addEventListener('click', closeFilterBar);
  document.getElementById('btnFbReset').addEventListener('click', resetFilterBar);
  document.getElementById('btnFbApply').addEventListener('click', applyFilterBar);
  // Operator-change + clear-row need delegation since rows are rebuilt
  // on every open.
  document.getElementById('fbRows').addEventListener('change', (e) => {
    const sel = e.target.closest('select[data-fb="op"]');
    if (sel) onFilterBarOpChange(sel);
  });
  document.getElementById('fbRows').addEventListener('keydown', (e) => {
    if (e.key === 'Enter') applyFilterBar();
  });
  document.getElementById('btnFioriCols').addEventListener('click', applyFioriCols);
  document.getElementById('btnFioriFilter').addEventListener('click', applyFioriFilter);
  document.getElementById('btnVlClose').addEventListener('click', closeValueListPicker);
  document.getElementById('btnVlFetch').addEventListener('click', fetchValueListRows);
  document.getElementById('vlFilter').addEventListener('keydown', (e) => {
    if (e.key === 'Enter') fetchValueListRows();
  });
  document.getElementById('vlSearch').addEventListener('keydown', (e) => {
    if (e.key === 'Enter') fetchValueListRows();
  });
  updateSapViewToggleState();
  document.getElementById('btnSapViewWarningsDismiss').addEventListener('click', () => showSapViewWarnings([]));
  document.getElementById('btnHistoryToggle').addEventListener('click', () => {
    const panel = document.getElementById('historyPanel');
    const tab = getActiveTab();
    if (panel.classList.contains('hidden')) {
      renderHistoryPanel(tab);
      panel.classList.remove('hidden');
      if (tab) tab._historyVisible = true;
    } else {
      panel.classList.add('hidden');
      if (tab) tab._historyVisible = false;
    }
  });
  document.getElementById('btnAddTab').addEventListener('click', () => {
    saveCurrentTabState();
    addTab({ title: 'New Tab', profile: state.currentProfile });
  });
  document.getElementById('btnModalClose').addEventListener('click', hideAddProfileModal);
  document.getElementById('btnCancel').addEventListener('click', hideAddProfileModal);
  document.getElementById('btnSave').addEventListener('click', saveProfileModal);
  document.getElementById('btnTest').addEventListener('click', testProfileModal);
  document.getElementById('mpAuthMode').addEventListener('change', updateAuthModeFields);

  // Filter tooltip click → apply filter
  document.getElementById('filterTooltip').addEventListener('click', applyFilterFromTooltip);

  // Hide filter tooltip when clicking elsewhere
  document.addEventListener('click', (e) => {
    if (e.target.id !== 'filterTooltip' && !e.target.closest('#filterTooltip')) {
      hideFilterTooltip();
    }
  });

  // ── Global event delegation ──
  document.addEventListener('click', (e) => {
    const el = e.target.closest('[data-action]');
    if (!el) return;
    const action = el.dataset.action;

    if (action === 'select') {
      addToSelect(el.dataset.field);
    } else if (action === 'expand') {
      addToExpand(el.dataset.field);
    } else if (action === 'nested') {
      showNestedData(el.dataset.key, el.dataset.col);
    } else if (action === 'close-nested') {
      const p = document.getElementById('nestedPanel');
      if (p) p.remove();
    } else if (action === 'switch-tab') {
      // Don't switch if the close button was clicked
      if (e.target.closest('[data-action="close-tab"]')) return;
      saveCurrentTabState();
      switchTab(el.dataset.tabId);
    } else if (action === 'close-tab') {
      e.stopPropagation();
      saveCurrentTabState();
      closeTab(el.dataset.tabId);
    } else if (action === 'copy-col') {
      e.stopPropagation();
      copyColumnValues(el.dataset.col);
    } else if (action === 'copy-row') {
      e.stopPropagation();
      copyRowAsJson(el.dataset.key);
    } else if (action === 'cell-click') {
      const col = el.dataset.cellCol;
      const val = el.dataset.cellVal;
      const rect = el.getBoundingClientRect();
      showFilterTooltip(col, val, rect.left, rect.bottom);
    } else if (action === 'replay-history') {
      replayHistory(parseInt(el.dataset.idx));
    } else if (action === 'clear-history') {
      const tab = getActiveTab();
      if (tab) { tab.queryHistory = []; renderHistoryPanel(tab); }
    } else if (action === 'select-trace') {
      const tab = getActiveTab();
      if (!tab) return;
      tab.selectedTraceId = parseInt(el.dataset.traceId, 10);
      tab._traceBodyExpanded = false;
      renderTraceInspector(tab);
    } else if (action === 'select-trace-subtab') {
      const tab = getActiveTab();
      if (!tab) return;
      tab._traceSubTab = el.dataset.subtab === 'request' ? 'request' : 'response';
      // Collapse the body expansion when switching sub-tabs — fresh
      // context means the user probably wants the short view first.
      tab._traceBodyExpanded = false;
      renderTraceDetail(tab);
    } else if (action === 'toggle-trace-body') {
      const tab = getActiveTab();
      if (!tab) return;
      tab._traceBodyExpanded = !tab._traceBodyExpanded;
      renderTraceDetail(tab);
    } else if (action === 'copy-trace-curl') {
      copySelectedTraceAsCurl();
    } else if (action === 'copy-trace-request-body') {
      copySelectedTraceRequestBody();
    } else if (action === 'copy-trace-response-body') {
      copySelectedTraceResponseBody();
    } else if (action === 'value-list') {
      openValueListPicker(el.dataset.prop);
    } else if (action === 'vl-select-variant') {
      const idx = parseInt(el.dataset.variantIndex, 10);
      if (!isNaN(idx)) selectVariant(idx);
    } else if (action === 'fb-clear-row') {
      const row = el.closest('[data-fb-row]');
      if (!row) return;
      row.querySelectorAll('input[data-fb="value"], input[data-fb="value-high"]').forEach(i => i.value = '');
      const sel = row.querySelector('select[data-fb="op"]');
      if (sel) {
        sel.value = 'eq';
        onFilterBarOpChange(sel);
      }
    } else if (action === 'ai-toggle-ns') {
      const ns = el.dataset.ns;
      if (!ns) return;
      if (_aiActiveNamespaces.has(ns)) {
        _aiActiveNamespaces.delete(ns);
      } else {
        _aiActiveNamespaces.add(ns);
      }
      renderAnnotationInspector();
    } else if (action === 'vl-pick') {
      pickValueListRow(parseInt(el.dataset.row, 10));
    } else if (action === 'selection-field') {
      const name = el.dataset.name;
      if (!name) return;
      if (e.shiftKey) {
        // Shift-click: append to $select instead of $filter. Dedupe
        // against whatever is already there so repeated clicks don't
        // bloat the list.
        const input = document.getElementById('qSelect');
        const current = (input.value || '').trim();
        const tokens = current ? current.split(',').map(s => s.trim()).filter(Boolean) : [];
        if (!tokens.includes(name)) tokens.push(name);
        input.value = tokens.join(',');
        input.focus();
        return;
      }
      // Plain click: seed $filter with "<name> eq ''" so the user can
      // complete the literal. Preserve anything already there via `and`.
      const input = document.getElementById('qFilter');
      const current = (input.value || '').trim();
      const snippet = `${name} eq ''`;
      input.value = current ? `${current} and ${snippet}` : snippet;
      input.focus();
      // Caret inside the trailing quotes so typing fills the value.
      const caret = input.value.lastIndexOf("''") + 1;
      if (caret > 0) input.setSelectionRange(caret, caret);
    } else if (action === 'back-to-services') {
      document.getElementById('serviceInput').value = '';
      const tab = getActiveTab();
      if (tab) tab._serviceInput = '';
      searchServices(state.lastSearchQuery === '' ? '' : state.lastSearchQuery);
    } else if (action === 'pick-service') {
      try {
        const svc = JSON.parse(el.dataset.svc || '{}');
        pickService(svc);
      } catch { /* ignore parse error */ }
    } else if (action === 'select-entity') {
      document.querySelectorAll('.sidebar-item').forEach(s => s.classList.remove('active'));
      el.classList.add('active');
      selectEntity(el.dataset.entityName, el);
    } else if (action === 'toggle-favorite') {
      e.stopPropagation();
      // Pull the full service object from the parent sidebar item so we store
      // {technical_name, title, version, ...} — not just the name.
      const parent = el.closest('[data-svc]');
      let svc;
      try { svc = parent ? JSON.parse(parent.dataset.svc) : { technical_name: el.dataset.svcName }; }
      catch { svc = { technical_name: el.dataset.svcName }; }
      toggleFavorite(svc, el);
    }
  });
});
