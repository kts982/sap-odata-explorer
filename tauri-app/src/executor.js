// ── Query execution + stats bar + URL builder ──
//
// `executeQuery` is the main "run query" entry point — wired to the
// Run / JSON buttons, Ctrl+Enter, and replay-from-history. It wires up
// the SAP-view pre-flight, runs the request via `timedInvoke`, hands
// off to renderResults / renderJson for the visible output, and
// records the run in the per-tab history.
//
// `readQueryParams` turns the query bar into request parameters; Run,
// "copy URL" and "copy CLI" all read it, so the three agree on what a
// filled key field means. `buildODataUrl` is consumed by the "Copy URL"
// feature in app.js's click delegate. `showStatsBar` / `hideStatsBar` /
// `formatBytes` are the bottom-bar stats helpers.
//
// All imports flow downward — no circular back to app.js.

import { state } from './state.js';
import { setStatus } from './status.js';
import { safeHtml } from './html.js';
import { tabScope, timedInvoke } from './api.js';
import { getActiveTab, currentDescribeInfo } from './tabs.js';
import { showSapViewWarnings, validateQueryRestrictions } from './query.js';
import { isBrowserAuthProfile, browserAuthMessage } from './auth.js';
import { addToHistory } from './history.js';
import { renderJson, renderResults, extractRows } from './results.js';
import { cacheQueryResult, clearQueryResultCache } from './resultCache.js';
import {
  normalizeEntityKey,
  effectiveQueryParams,
  buildQueryUrl,
  serviceODataVersion,
} from './format.js';

export function buildODataUrl(params) {
  return buildQueryUrl(state.currentServicePath, params);
}

// The query bar as request parameters. With a key the request reads one
// entity: the list options keep their values in the bar but are not sent
// (see effectiveQueryParams). `opts.skiptoken` follows a server-driven
// next link (pager "next").
export function readQueryParams(opts = {}) {
  const tab = getActiveTab();
  const value = id => document.getElementById(id).value;
  return effectiveQueryParams({
    entity_set: state.currentEntitySet,
    key: normalizeEntityKey(
      value('qKey'),
      currentDescribeInfo(tab),
      state.currentEntitySet,
      serviceODataVersion(tab),
    ),
    select:  value('qSelect')  || null,
    filter:  value('qFilter')  || null,
    expand:  value('qExpand')  || null,
    orderby: value('qOrderby') || null,
    top:     parseInt(value('qTop'))  || null,
    skip:    parseInt(value('qSkip')) || null,
    count:   document.getElementById('qCount').checked,
    skiptoken: opts.skiptoken || null,
  });
}

export async function executeQuery(asJson = false, opts = {}) {
  if (!state.currentProfile || !state.currentServicePath || !state.currentEntitySet) {
    setStatus('Select a profile, service, and entity set first');
    return;
  }

  const params = readQueryParams(opts);
  // Show the key as it is sent (a plain value may have been quoted).
  if (params.key) document.getElementById('qKey').value = params.key;

  // SAP View pre-flight: when enabled, surface restriction warnings
  // without blocking. Annotations describe what Fiori would do; SAP
  // servers are often more permissive than the metadata claims, so the
  // server's response is the final word. We just make the context
  // visible before hitting send.
  if (state.sapViewEnabled) {
    const info = currentDescribeInfo(getActiveTab());
    if (info && info.name) {
      showSapViewWarnings(validateQueryRestrictions(params, info));
    } else {
      showSapViewWarnings([]);
    }
  } else {
    showSapViewWarnings([]);
  }

  setStatus(params.key
    ? `Reading ${state.currentEntitySet}(${params.key})...`
    : `Querying ${state.currentEntitySet}...`);
  const queryStart = performance.now();
  const scope = tabScope('query');

  try {
    const data = await timedInvoke('run_query', {
      profileName: state.currentProfile,
      servicePath: state.currentServicePath,
      params,
    });
    if (!scope.active()) return;

    const elapsed = Math.round(performance.now() - queryStart);
    const tab = getActiveTab();
    cacheQueryResult(tab, data, params, elapsed, asJson);

    if (asJson) {
      renderJson(data);
      hideStatsBar();
    } else {
      renderResults(data, elapsed, params);
    }

    // Record in history
    if (tab && !asJson) {
      const rows = extractRows(data);
      const rowCount = rows ? rows.length : 0;
      addToHistory(tab, params, rowCount, elapsed);
    }
  } catch (e) {
    if (!scope.active()) return;
    setStatus('Query error: ' + e);
    hideStatsBar();
    clearQueryResultCache(getActiveTab());
    state.expandedDataStore = {};
    state.lastResultRows = null;
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
