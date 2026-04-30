// ── Per-tab query history ──
//
// `tab.queryHistory` is an in-memory ring of the last 20 queries this
// tab ran. `addToHistory` is called from executor.js after a
// successful run; `replayHistory` puts a recorded entry back into the
// query bar and re-fires the query.
//
// Circular pair with executor.js (executor.js → addToHistory; this
// module → executeQuery). Sibling-module circularity is fine — ESM
// resolves it because the bindings are referenced inside function
// bodies only.

import { state } from './state.js';
import { safeHtml, raw } from './html.js';
import { getActiveTab } from './tabs.js';
import { executeQuery } from './executor.js';

export function addToHistory(tab, params, rowCount, elapsed) {
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

export function renderHistoryPanel(tab) {
  const panel = document.getElementById('historyPanel');
  if (!tab || tab.queryHistory.length === 0) {
    panel.innerHTML = '<div class="px-4 py-3 text-[11px] text-ox-dim font-mono">No history yet</div>';
    return;
  }
  const rows = tab.queryHistory.map((h, i) => {
    const time = h.ts.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
    return safeHtml`
      <div class="history-item" data-action="replay-history" data-idx="${i}">
        <span class="text-ox-amber shrink-0">${h.entitySet}</span>
        <span class="text-ox-dim flex-1 truncate">${h.summary}</span>
        <span class="text-ox-dim shrink-0">${h.rowCount}r</span>
        <span class="text-ox-dim shrink-0">${h.elapsed}ms</span>
        <span class="text-ox-dim shrink-0">${time}</span>
      </div>`;
  }).join('');
  const html = safeHtml`
    <div class="flex items-center justify-between px-3 py-1 border-b border-ox-border">
      <span class="text-[9px] uppercase tracking-widest text-ox-dim font-medium">Query History</span>
      <button data-action="clear-history" class="text-[10px] text-ox-dim hover:text-ox-red px-1 transition-colors">clear</button>
    </div>
    ${raw(rows)}`;
  panel.innerHTML = html;
}

export function replayHistory(idx) {
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
