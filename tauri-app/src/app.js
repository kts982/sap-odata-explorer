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
import {
  propertyFlagHints,
  validateQueryRestrictions,
  showSapViewWarnings,
  renderSelectionFieldsBar,
  openFilterBar,
  closeFilterBar,
  resetFilterBar,
  onFilterBarOpChange,
  applyFilterBar,
} from './query.js';
import {
  renderFioriColsButton,
  renderFioriFilterButton,
  applyFioriFilter,
  applyFioriCols,
  renderFioriReadinessPanel,
} from './fiori.js';
import {
  openValueListPicker,
  selectVariant,
  closeValueListPicker,
  fetchValueListRows,
  pickValueListRow,
} from './valueList.js';
import {
  renderTraceSummary,
  renderTraceInspector,
  renderTraceDetail,
  updateTraceToggleState,
  toggleTraceInspector,
  hideTraceInspector,
  copySelectedTraceAsCurl,
  copySelectedTraceRequestBody,
  copySelectedTraceResponseBody,
} from './trace.js';
import {
  renderDescribe,
  hideDescribe,
  addToSelect,
  addToExpand,
} from './describe.js';
import {
  renderAnnotationBadge,
  openAnnotationInspector,
  closeAnnotationInspector,
  renderAnnotationInspector,
  toggleAnnotationNamespace,
} from './annotations.js';
import { copyToClipboard } from './clipboard.js';
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



// ══════════════════════════════════════════════════════════════
// ── RESULTS RENDERING ──
// ══════════════════════════════════════════════════════════════

export function extractRows(data) {
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
      toggleAnnotationNamespace(ns);
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
