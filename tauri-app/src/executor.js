// ── Query execution + stats bar + URL builder ──
//
// `executeQuery` is the main "run query" entry point — wired to the
// Run / JSON buttons, Ctrl+Enter, and replay-from-history. It wires up
// the SAP-view pre-flight, runs the request via `timedInvoke`, hands
// off to renderResults / renderJson for the visible output, and
// records the run in the per-tab history.
//
// `buildODataUrl` is consumed by the "Copy URL" feature in app.js's
// click delegate. `showStatsBar` / `hideStatsBar` / `formatBytes` are
// the bottom-bar stats helpers.
//
// One temporary circular import: `renderJson` + `renderResults` are
// still in app.js until 6e extracts results.js. After 6e, those move
// to results.js and this module's app.js import goes away.

import { state } from './state.js';
import { setStatus } from './status.js';
import { safeHtml } from './html.js';
import { tabScope, timedInvoke } from './api.js';
import { getActiveTab } from './tabs.js';
import { showSapViewWarnings, validateQueryRestrictions } from './query.js';
import { isBrowserAuthProfile, browserAuthMessage } from './auth.js';
import { addToHistory } from './history.js';
import { renderJson, renderResults, extractRows } from './app.js';

export function buildODataUrl(params) {
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

export async function executeQuery(asJson = false) {
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

export function showStatsBar(rowCount, sizeBytes, elapsedMs) {
  document.getElementById('statRows').textContent = `${rowCount} row${rowCount !== 1 ? 's' : ''}`;
  document.getElementById('statSize').textContent = formatBytes(sizeBytes);

  let timingClass = 'timing-fast';
  if (elapsedMs >= 2000) timingClass = 'timing-slow';
  else if (elapsedMs >= 500) timingClass = 'timing-ok';

  document.getElementById('statTiming').innerHTML =
    safeHtml`<span class="${timingClass}">${elapsedMs}ms</span>`;
  document.getElementById('statsBar').classList.remove('hidden');
}

export function hideStatsBar() {
  document.getElementById('statsBar').classList.add('hidden');
}

function formatBytes(bytes) {
  if (bytes < 1024) return bytes + ' B';
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
  return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
}
