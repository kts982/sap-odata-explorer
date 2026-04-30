// ── Tab system ──
//
// Each tab carries its own copy of the data the UI cares about: the
// active service + entity + query bar values + cached results +
// HTTP trace entries + history. `state` (state.js) holds *active-tab
// mirrors* of the most-frequently-read fields; `restoreTabUI` (still in
// app.js — see below) syncs those mirrors when the user switches.
//
// `restoreTabUI` deliberately stays in app.js. It touches every UI
// surface (services / describe / query bar / results / traces / history
// / annotations / stats) and pulling it in here would force callbacks
// or upward imports back to most of the codebase. The plan is to leave
// the cross-module orchestration in app.js until the rest of the
// modules are split out and `restoreTabUI`'s sites have natural homes;
// only then is it safe to lift.
//
// Two named imports cross module boundaries: `restoreTabUI` from
// app.js (still circular until the rest of the orchestration splits
// out), and `searchServices` from services.js. ESM resolves the
// app.js circle because the binding is only referenced inside a
// function body, not at top-level evaluation.

import { state } from './state.js';
import { restoreTabUI } from './app.js';
import { searchServices } from './services.js';

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
  const tab = createTab(opts);
  state.tabs.push(tab);
  renderTabBar();
  switchTab(tab.id);
  // If there's an active profile, auto-load services (shows favorites at top)
  if (state.currentProfile && state.cachedServices) {
    tab.profile = state.currentProfile;
    tab.cachedServices = state.cachedServices;
    searchServices('');
  }
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
