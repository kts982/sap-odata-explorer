// ── Per-tab query result cache ──
//
// Results rendering depends on global display preferences such as SAP View,
// so cached DOM HTML can go stale when a tab is restored. Keep the raw query
// payload instead and let tabs.js re-render it against the current settings.

export function cacheQueryResult(tab, data, params, elapsedMs, asJson) {
  if (!tab) return;
  tab._lastQueryData = data;
  tab._lastQueryParams = params ? { ...params } : null;
  tab._lastQueryElapsed = elapsedMs || 0;
  tab._lastQueryAsJson = asJson === true;
  tab._resultsHtml = undefined;
}

export function clearQueryResultCache(tab) {
  if (!tab) return;
  tab._lastQueryData = undefined;
  tab._lastQueryParams = undefined;
  tab._lastQueryElapsed = undefined;
  tab._lastQueryAsJson = undefined;
  tab._resultsHtml = undefined;
  tab._expandedDataStore = {};
  tab._lastResultRows = null;
}

export function hasCachedQueryResult(tab) {
  return !!tab && tab._lastQueryData !== undefined;
}
