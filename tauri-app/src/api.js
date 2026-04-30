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
// The trace-application path imports four helpers from app.js — getTab,
// ensureTraceSelection, renderTraceSummary, renderTraceInspector. That
// is a circular import (app.js also imports from api.js); ES modules
// resolve it correctly because the helpers are only invoked inside
// function bodies, never at top-level evaluation. Future batches will
// lift those helpers into tabs.js / a trace-rendering module and the
// circle will close.

import { invoke } from './vendor/tauri-core.js';
import { state } from './state.js';
import { setTime, showSpinner, hideSpinner } from './status.js';
import {
  getTab,
  ensureTraceSelection,
  renderTraceSummary,
  renderTraceInspector,
} from './app.js';

export function tabScope() {
  const originTabId = state.activeTabId;
  return {
    originTabId,
    active: () => state.activeTabId === originTabId,
  };
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
