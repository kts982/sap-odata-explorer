// ── Tauri IPC wrapper layer ──
//
// Wraps `invoke` with two cross-cutting concerns:
//   1. Spinner + elapsed-ms status. Every command toggles the global
//      spinner; commands that touch the network also surface the round-
//      trip time in the status bar.
//   2. Tab correlation. SAP requests can take seconds. If the user
//      switches tabs mid-flight, DOM writes intended for the origin tab
//      would land in whichever tab happens to be active when the await
//      resumes. `tabScope()` snapshots the origin tab id so callers can
//      bail out via `if (!scope.active()) return;` and re-trigger the
//      action (via the tab's cached state) when the user comes back.
//      `timedInvoke()` correlates the trace by the same originTabId so
//      the trace inspector renders on the *originating* tab even after
//      the user has moved on.
//
// All imports flow downward — no circular back to app.js.

import { invoke } from './vendor/tauri-core.js';
import { state } from './state.js';
import { setTime, showSpinner, hideSpinner } from './status.js';
import { getTab } from './tabs.js';
import {
  ensureTraceSelection,
  renderTraceSummary,
  renderTraceInspector,
} from './trace.js';

// Request context levels, outermost first. A tab switch is not the only
// way a response goes stale: within one tab, a newer request at the same
// level (re-run with a narrower filter) or a context change above it
// (another entity set, a pasted service path, another profile) must win
// over a slower older response. Starting work at a level bumps that
// level's generation and every inner one; `scope.active()` is false once
// the generation it captured has moved on.
//   service — catalog search and service load (they replace each other)
//   entity  — describe
//   query   — run_query
const REQUEST_LEVELS = ['service', 'entity', 'query'];

function bumpGenerations(tab, level) {
  if (!tab._requestGen) tab._requestGen = {};
  const from = REQUEST_LEVELS.indexOf(level);
  for (const l of REQUEST_LEVELS.slice(from < 0 ? 0 : from)) {
    tab._requestGen[l] = (tab._requestGen[l] || 0) + 1;
  }
  return tab._requestGen[level];
}

export function tabScope(level) {
  const originTabId = state.activeTabId;
  const tab = getTab(originTabId);
  const gen = level && tab ? bumpGenerations(tab, level) : null;
  return {
    originTabId,
    active: () =>
      state.activeTabId === originTabId &&
      (gen === null || tab._requestGen[level] === gen),
  };
}

// Make every in-flight request of this tab stale (profile switch).
export function invalidateTabRequests(tab) {
  if (tab) bumpGenerations(tab, REQUEST_LEVELS[0]);
}

export async function timedInvoke(cmd, args) {
  showSpinner();
  const start = performance.now();
  const originTabId = state.activeTabId;
  try {
    const result = await invoke(cmd, args);
    setTime(Math.round(performance.now() - start));
    // Commands that touch the network return { data, trace }. Legacy commands
    // still return their value directly.
    if (result && typeof result === 'object' && 'data' in result && Array.isArray(result.trace)) {
      applyTraceToTab(originTabId, result.trace);
      return result.data;
    }
    return result;
  } catch (err) {
    // Network commands serialize errors as { message, trace } — apply the trace
    // and re-throw the plain message so callers keep the string-based API.
    if (err && typeof err === 'object' && 'message' in err && Array.isArray(err.trace)) {
      applyTraceToTab(originTabId, err.trace);
      throw err.message;
    }
    throw err;
  } finally {
    hideSpinner();
  }
}

export function applyTraceToTab(tabId, trace) {
  const tab = getTab(tabId);
  if (!tab) return;
  tab.httpTraceEntries = Array.isArray(trace) ? trace : [];
  if (!tab.httpTraceEntries.some(entry => entry.id === tab.selectedTraceId)) {
    tab.selectedTraceId = null;
  }
  ensureTraceSelection(tab);
  if (tab.id === state.activeTabId) {
    renderTraceSummary(tab);
    if (tab._traceVisible) {
      renderTraceInspector(tab);
    }
  }
}

export function updateServicePathBar(tab) {
  const bar = document.getElementById('servicePathBar');
  if (tab && tab.servicePath) {
    document.getElementById('servicePathText').textContent = tab.servicePath;
    const verEl = document.getElementById('servicePathVersion');
    if (tab.serviceVersion) {
      verEl.textContent = tab.serviceVersion;
      verEl.className = 'text-[10px] px-1 py-px rounded-sm font-mono ' +
        (tab.serviceVersion === 'V4' ? 'badge-v4' : 'badge-v2');
      verEl.style.display = '';
    } else {
      verEl.style.display = 'none';
    }
    bar.classList.add('visible');
  } else {
    bar.classList.remove('visible');
  }
}
