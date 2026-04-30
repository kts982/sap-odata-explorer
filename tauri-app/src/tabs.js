// ── Tab system ──
//
// Each tab carries its own copy of the data the UI cares about: the
// active service + entity + query bar values + cached results +
// HTTP trace entries + history. `state` (state.js) holds *active-tab
// mirrors* of the most-frequently-read fields; `restoreTabUI` syncs
// those mirrors when the user switches.
//
// `restoreTabUI` is the cross-module orchestrator: it pokes at every
// renderer to put the new tab's UI back on screen. That makes tabs.js
// a sibling-circular partner with auth / api / history / trace /
// services. The cycles are benign — every cross-module call sits
// inside a function body, never at top-level evaluation, so ESM
// resolves the bindings lazily.

import { state } from './state.js';
import { resetResultsArea } from './services.js';
import { updateProfileAuthUi } from './auth.js';
import { updateServicePathBar } from './api.js';
import { renderHistoryPanel } from './history.js';
import {
  renderTraceSummary,
  renderTraceInspector,
  updateTraceToggleState,
} from './trace.js';
import { renderAnnotationBadge } from './annotations.js';
import { renderDescribe } from './describe.js';
import { renderJson, renderResults } from './results.js';
import { hasCachedQueryResult } from './resultCache.js';

export function createTab(opts = {}) {
  const id = 'tab_' + Date.now() + '_' + Math.random().toString(36).slice(2);
  return {
    id,
    title: opts.title || 'New Tab',
    // per-tab state
    profile: opts.profile || null,
    servicePath: opts.servicePath || null,
    serviceVersion: opts.serviceVersion || null,   // 'V2' | 'V4' | null
    entitySet: opts.entitySet || null,
    entitySets: [],
    cachedServices: null,
    lastSearchQuery: null,
    // query history (last 20, in-memory)
    queryHistory: [],
    // last query params (for "history" re-use)
    lastParams: null,
    httpTraceEntries: [],
    selectedTraceId: null,
    _traceVisible: false,
    _traceSubTab: 'response',
    annotationSummary: null,
  };
}

export function getTab(id) {
  return state.tabs.find(t => t.id === id) || null;
}

export function getActiveTab() {
  return getTab(state.activeTabId);
}

export function addTab(opts = {}) {
  // New tabs land in the "Select profile..." default — no inherit
  // from the active tab. Same mental model as opening a new browser
  // tab: a blank canvas, the user picks a profile per tab. The
  // earlier inherit attempt was preexisting dead code (the order of
  // restoreTabUI's state writes vs. the inherit guard meant it never
  // fired) and "fixing" it surfaced a second gap — favorites-only
  // sidebar only renders on a real profile-select event, not on a
  // programmatic dropdown set during restoreTabUI. Cleaner to just
  // not inherit.
  const tab = createTab(opts);
  state.tabs.push(tab);
  renderTabBar();
  switchTab(tab.id);
  return tab;
}

export function closeTab(id) {
  if (state.tabs.length <= 1) return; // keep at least 1
  const idx = state.tabs.findIndex(t => t.id === id);
  if (idx === -1) return;
  state.tabs.splice(idx, 1);
  if (state.activeTabId === id) {
    const next = state.tabs[Math.min(idx, state.tabs.length - 1)];
    renderTabBar();
    switchTab(next.id);
  } else {
    renderTabBar();
  }
}

export function switchTab(id) {
  state.activeTabId = id;
  renderTabBar();
  restoreTabUI();
}

export function renderTabBar() {
  const bar = document.getElementById('tabBar');
  const addBtn = document.getElementById('btnAddTab');
  // Remove all tab elements (not the add button)
  [...bar.querySelectorAll('.tab-item')].forEach(el => el.remove());

  for (const tab of state.tabs) {
    const el = document.createElement('div');
    el.className = 'tab-item' + (tab.id === state.activeTabId ? ' active' : '');
    el.dataset.tabId = tab.id;

    const titleEl = document.createElement('span');
    titleEl.className = 'tab-title';
    titleEl.textContent = tab.title;

    const closeEl = document.createElement('span');
    closeEl.className = 'tab-close';
    closeEl.textContent = '×';
    closeEl.dataset.action = 'close-tab';
    closeEl.dataset.tabId = tab.id;

    el.appendChild(titleEl);
    if (state.tabs.length > 1) el.appendChild(closeEl);
    el.dataset.action = 'switch-tab';

    bar.insertBefore(el, addBtn);
  }
}

/** Save current UI state into the active tab, then restore the new tab's UI */
export function saveCurrentTabState() {
  const tab = getActiveTab();
  if (!tab) return;
  // Save query bar values
  tab._qSelect  = document.getElementById('qSelect').value;
  tab._qFilter  = document.getElementById('qFilter').value;
  tab._qExpand  = document.getElementById('qExpand').value;
  tab._qOrderby = document.getElementById('qOrderby').value;
  tab._qTop     = document.getElementById('qTop').value;
  tab._qSkip    = document.getElementById('qSkip').value;
  // Save results HTML and stats
  tab._resultsHtml = document.getElementById('resultsArea').innerHTML;
  tab._statsVisible = !document.getElementById('statsBar').classList.contains('hidden');
  tab._statRows  = document.getElementById('statRows').textContent;
  tab._statSize  = document.getElementById('statSize').textContent;
  tab._statTiming = document.getElementById('statTiming').innerHTML;
  tab._describePanelHidden = document.getElementById('describePanel').classList.contains('hidden');
  tab._describeTitle = document.getElementById('entityTitle').textContent;
  tab._describeHtml = document.getElementById('describeContent').innerHTML;
  tab._queryBarHidden = document.getElementById('queryBar').classList.contains('hidden');
  tab._queryEntitySet = document.getElementById('queryEntitySet').textContent;
  tab._historyVisible = !document.getElementById('historyPanel').classList.contains('hidden');
  tab._traceVisible = !document.getElementById('traceInspectorPanel').classList.contains('hidden');
  tab._sidebarTitle = document.getElementById('sidebarTitle').textContent;
  tab._sidebarCount = document.getElementById('sidebarCount').textContent;
  tab._sidebarHtml = document.getElementById('entityList').innerHTML;
  tab._serviceInput = document.getElementById('serviceInput').value;
  // Save backing data for copy/expand
  tab._expandedDataStore = { ...state.expandedDataStore };
  tab._lastResultRows = state.lastResultRows;
}

export function restoreTabUI() {
  const tab = getActiveTab();
  if (!tab) return;

  // Sync active-tab mirrors that other modules read off of state.
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

  // Describe panel. SAP View is a *global* preference, so we re-render
  // from cached describe info rather than restoring the cached HTML —
  // that way switching tabs reflects the current state.sapViewEnabled,
  // not the value at the time this tab was last rendered.
  if (tab._describePanelHidden === false) {
    document.getElementById('describePanel').classList.remove('hidden');
    if (tab._lastDescribeInfo) {
      renderDescribe(tab._lastDescribeInfo);
    } else {
      document.getElementById('entityTitle').textContent = tab._describeTitle || '';
      document.getElementById('describeContent').innerHTML = tab._describeHtml || '';
    }
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

  const hasResultCache = hasCachedQueryResult(tab);

  // Stats bar. Cached raw query results re-render below and own the
  // stats state; the saved DOM text is only a fallback for older tabs.
  if (!hasResultCache && tab._statsVisible) {
    document.getElementById('statRows').textContent = tab._statRows || '';
    document.getElementById('statSize').textContent = tab._statSize || '';
    document.getElementById('statTiming').innerHTML = tab._statTiming || '';
    document.getElementById('statsBar').classList.remove('hidden');
  } else if (!hasResultCache) {
    document.getElementById('statsBar').classList.add('hidden');
  }

  // Results. Prefer raw query data so SAP View and formatter changes are
  // reflected on restore instead of replaying stale DOM HTML.
  if (hasResultCache) {
    if (tab._lastQueryAsJson) {
      renderJson(tab._lastQueryData);
    } else {
      renderResults(tab._lastQueryData, tab._lastQueryElapsed, tab._lastQueryParams);
    }
  } else if (tab._resultsHtml !== undefined) {
    document.getElementById('resultsArea').innerHTML = tab._resultsHtml;
  } else {
    resetResultsArea();
  }
}

// All sidebar items (back link, service items, star buttons, entity
// items) are handled by document-level delegation only — nothing to
// re-attach. Kept as a named function so restoreTabUI's intent is
// readable: "rendered HTML, now re-bind anything imperative."
function reattachSidebarHandlers() {
  /* intentionally empty */
}
