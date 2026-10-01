// ── Query history (persistent, per profile + service) ──
//
// Runs are kept in localStorage per (profile, service path), so history
// survives restarts and follows the service rather than the tab. Only the
// query parameters are stored — never response data. Re-running the same
// query refreshes its entry instead of adding a duplicate. ★ pins an
// entry: pinned entries are listed first and never age out; "clear"
// keeps them. `addToHistory` is called from executor.js after a
// successful run; `replayHistory` puts an entry back into the query bar
// and re-runs it.
//
// Circular pair with executor.js (executor.js → addToHistory; this
// module → executeQuery). Sibling-module circularity is fine — ESM
// resolves it because the bindings are referenced inside function
// bodies only.

import { state } from './state.js';
import { safeHtml, raw } from './html.js';
import { getActiveTab } from './tabs.js';
import { executeQuery } from './executor.js';
import { selectEntity } from './services.js';
import { setStatus } from './status.js';

const MAX_UNPINNED = 50;
const STORED_PARAMS = ['select', 'filter', 'expand', 'orderby', 'top', 'skip', 'count', 'skiptoken'];

function storageKey(tab) {
  return tab && tab.profile && tab.servicePath
    ? `ox_history_v1::${tab.profile}::${tab.servicePath}`
    : null;
}

function loadHistory(tab) {
  const key = storageKey(tab);
  if (!key) return [];
  try {
    const list = JSON.parse(localStorage.getItem(key) || '[]');
    return Array.isArray(list) ? list : [];
  } catch {
    return [];
  }
}

function saveHistory(tab, list) {
  const key = storageKey(tab);
  if (!key) return;
  const pinned = list.filter(h => h.pinned);
  const recent = list.filter(h => !h.pinned).slice(0, MAX_UNPINNED);
  // Keep stored order: newest first, pinned wherever they fell.
  const keep = new Set([...pinned, ...recent]);
  try {
    localStorage.setItem(key, JSON.stringify(list.filter(h => keep.has(h))));
  } catch {
    setStatus('Could not save query history (local storage full?)');
  }
}

function sameQuery(a, b) {
  return a.entitySet === b.entitySet
    && STORED_PARAMS.every(p => (a.params[p] ?? null) === (b.params[p] ?? null));
}

export function addToHistory(tab, params, rowCount, elapsed) {
  const stored = {};
  for (const p of STORED_PARAMS) stored[p] = params[p] ?? null;
  const entry = {
    ts: new Date().toISOString(),
    entitySet: params.entity_set,
    params: stored,
    rowCount,
    elapsed,
    pinned: false,
  };
  const list = loadHistory(tab);
  const existing = list.findIndex(h => sameQuery(h, entry));
  if (existing >= 0) {
    entry.pinned = list[existing].pinned;
    list.splice(existing, 1);
  }
  list.unshift(entry);
  saveHistory(tab, list);
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
  if (params.count)   parts.push('$count');
  return parts.join(' · ') || '(no params)';
}

function formatWhen(iso) {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  const today = new Date().toDateString() === d.toDateString();
  return today
    ? d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })
    : d.toLocaleDateString([], { month: 'short', day: 'numeric' });
}

export function renderHistoryPanel(tab) {
  const panel = document.getElementById('historyPanel');
  const list = loadHistory(tab);
  if (list.length === 0) {
    panel.innerHTML = '<div class="px-4 py-3 text-[11px] text-ox-dim font-mono">No history yet for this service</div>';
    return;
  }
  // Pinned first, each group newest first; data-idx is the stored index.
  const order = [...list.keys()].sort((a, b) => Number(list[b].pinned) - Number(list[a].pinned));
  const rows = order.map(i => {
    const h = list[i];
    return safeHtml`
      <div class="history-item" data-action="replay-history" data-idx="${i}">
        <span class="shrink-0 cursor-pointer ${h.pinned ? 'text-ox-amber' : 'text-ox-dim hover:text-ox-amber'}" data-action="pin-history" data-idx="${i}" title="${h.pinned ? 'Unpin' : 'Pin — keep this query'}">${h.pinned ? '★' : '☆'}</span>
        <span class="text-ox-amber shrink-0">${h.entitySet}</span>
        <span class="text-ox-dim flex-1 truncate">${buildParamSummary(h.params)}</span>
        <span class="text-ox-dim shrink-0">${h.rowCount}r</span>
        <span class="text-ox-dim shrink-0">${h.elapsed}ms</span>
        <span class="text-ox-dim shrink-0">${formatWhen(h.ts)}</span>
      </div>`;
  }).join('');
  panel.innerHTML = safeHtml`
    <div class="flex items-center justify-between px-3 py-1 border-b border-ox-border">
      <span class="text-[9px] uppercase tracking-widest text-ox-dim font-medium">Query History — this service</span>
      <button data-action="clear-history" class="text-[10px] text-ox-dim hover:text-ox-red px-1 transition-colors" title="Remove unpinned entries">clear</button>
    </div>
    ${raw(rows)}`;
}

export function togglePinHistory(idx) {
  const tab = getActiveTab();
  const list = loadHistory(tab);
  if (!list[idx]) return;
  list[idx].pinned = !list[idx].pinned;
  saveHistory(tab, list);
  renderHistoryPanel(tab);
}

export function clearHistory(tab) {
  saveHistory(tab, loadHistory(tab).filter(h => h.pinned));
  renderHistoryPanel(tab);
}

export async function replayHistory(idx) {
  const tab = getActiveTab();
  if (!tab) return;
  const h = loadHistory(tab)[idx];
  if (!h) return;
  // Another entity set: select it so describe info (SAP View reshaping,
  // pre-flight warnings) matches the replayed query.
  if (h.entitySet && h.entitySet !== state.currentEntitySet) {
    const row = [...document.querySelectorAll('[data-action="select-entity"]')]
      .find(el => el.dataset.entityName === h.entitySet);
    await selectEntity(h.entitySet, row || null);
    if (getActiveTab() !== tab || state.currentEntitySet !== h.entitySet) return;
  }
  document.getElementById('qSelect').value  = h.params.select  || '';
  document.getElementById('qFilter').value  = h.params.filter  || '';
  document.getElementById('qExpand').value  = h.params.expand  || '';
  document.getElementById('qOrderby').value = h.params.orderby || '';
  document.getElementById('qTop').value     = h.params.top     || '20';
  document.getElementById('qSkip').value    = h.params.skip    || '';
  document.getElementById('qCount').checked = !!h.params.count;
  executeQuery(false, { skiptoken: h.params.skiptoken || null });
}
