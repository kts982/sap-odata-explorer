// ── Annotation badge + inspector ──
//
// Footer badge counts raw annotations from the active service and acts
// as the open trigger for the inspector modal. The inspector itself
// lazy-loads the full annotation list (cached per service path) and
// supports namespace toggles + free-text search.
//
// Module-private state: `_aiAnnotations` (current loaded list),
// `_aiActiveNamespaces` (filter set), `_aiCache` (per-service results).
// `toggleAnnotationNamespace` is exported so the document-level click
// delegate in app.js can flip a chip without reaching into the private
// set directly.
//
// All imports flow downward — no circular back to app.js.

import { state } from './state.js';
import { escapeHtml, safeHtml } from './html.js';
import { timedInvoke } from './api.js';

// Lazy-loaded dump of every raw annotation the parser captured. Good
// for answering "does this service declare X?" when the feature view
// doesn't surface it yet, or for grepping across namespaces. Cached
// per service path so reopening is instant.
let _aiAnnotations = [];
let _aiActiveNamespaces = new Set(); // empty = show all
const _aiCache = new Map();

export function renderAnnotationBadge(summary) {
  const el = document.getElementById('annotationBadge');
  if (!el) return;
  const total = summary && typeof summary.total === 'number' ? summary.total : 0;
  if (total === 0) {
    el.classList.add('hidden');
    el.textContent = '';
    el.title = '';
    return;
  }
  el.classList.remove('hidden');
  el.textContent = `${total} annotation${total === 1 ? '' : 's'}`;
  const byNs = summary.by_namespace || {};
  const lines = Object.entries(byNs)
    .sort(([, a], [, b]) => b - a)
    .map(([ns, count]) => `${ns}: ${count}`);
  el.title = lines.length
    ? `${lines.join('\n')}\n\nClick to open the annotation inspector`
    : `${total} annotations — click to inspect`;
}

export async function openAnnotationInspector() {
  const modal = document.getElementById('annotationInspectorModal');
  const subtitle = document.getElementById('aiSubtitle');
  const search = document.getElementById('aiSearch');
  const results = document.getElementById('aiResults');
  if (!modal) return;
  if (!state.currentProfile || !state.currentServicePath) return;
  modal.classList.remove('hidden');
  subtitle.textContent = state.currentServicePath;
  search.value = '';
  _aiActiveNamespaces = new Set();
  const cacheKey = `${state.currentProfile}::${state.currentServicePath}`;
  let annotations = _aiCache.get(cacheKey);
  if (!annotations) {
    results.innerHTML = '<div class="p-4 text-ox-dim text-[11px]">Loading annotations…</div>';
    try {
      annotations = await timedInvoke('get_annotations', {
        profileName: state.currentProfile,
        servicePath: state.currentServicePath,
      });
      _aiCache.set(cacheKey, annotations);
    } catch (e) {
      results.innerHTML = safeHtml`<div class="p-4 text-ox-red text-[11px]">Could not load annotations:\n${String(e)}</div>`;
      return;
    }
  }
  _aiAnnotations = Array.isArray(annotations) ? annotations : [];
  renderAnnotationInspector();
  setTimeout(() => search.focus(), 0);
}

export function closeAnnotationInspector() {
  document.getElementById('annotationInspectorModal').classList.add('hidden');
}

// Toggle a namespace chip in the inspector filter. Called from the
// click delegate in app.js (action=`ai-toggle-ns`). Encapsulates the
// private `_aiActiveNamespaces` set so callers don't reach into module
// internals.
export function toggleAnnotationNamespace(ns) {
  if (_aiActiveNamespaces.has(ns)) {
    _aiActiveNamespaces.delete(ns);
  } else {
    _aiActiveNamespaces.add(ns);
  }
  renderAnnotationInspector();
}

// Re-render the inspector's filtered table + namespace chips against
// the currently-loaded annotation list, the text filter, and the
// active namespace toggles.
export function renderAnnotationInspector() {
  const results = document.getElementById('aiResults');
  const nsBar = document.getElementById('aiNamespaceBar');
  const countEl = document.getElementById('aiCount');
  const searchEl = document.getElementById('aiSearch');
  const needle = (searchEl.value || '').trim().toLowerCase();
  // Build namespace list + counts from the FULL annotation set so the
  // chip bar doesn't flicker when filters toggle.
  const nsCounts = new Map();
  for (const a of _aiAnnotations) {
    nsCounts.set(a.namespace, (nsCounts.get(a.namespace) || 0) + 1);
  }
  const sortedNs = [...nsCounts.entries()].sort(([, a], [, b]) => b - a);
  nsBar.innerHTML = sortedNs
    .map(([ns, count]) => {
      const on = _aiActiveNamespaces.size === 0 || _aiActiveNamespaces.has(ns);
      const base = 'text-[10px] font-semibold tracking-wide px-1.5 py-0.5 rounded-sm transition-colors cursor-pointer';
      const onCls = 'text-ox-blue border border-ox-blue bg-ox-blue/10';
      const offCls = 'text-ox-dim border border-ox-border hover:text-ox-blue hover:border-ox-blue/50';
      const cls = on ? `${base} ${onCls}` : `${base} ${offCls}`;
      return `<button type="button" class="${cls}" data-action="ai-toggle-ns" data-ns="${escapeHtml(ns)}">${escapeHtml(ns)} · ${count}</button>`;
    })
    .join('');
  // Filter by namespace + text search.
  const filtered = _aiAnnotations.filter(a => {
    if (_aiActiveNamespaces.size > 0 && !_aiActiveNamespaces.has(a.namespace)) return false;
    if (needle) {
      const hay = [a.term, a.target, a.value || '', a.qualifier || '']
        .join(' ').toLowerCase();
      if (!hay.includes(needle)) return false;
    }
    return true;
  });
  countEl.textContent = `${filtered.length} of ${_aiAnnotations.length}`;
  if (filtered.length === 0) {
    results.innerHTML = '<div class="p-4 text-ox-dim text-[11px]">No matching annotations.</div>';
    return;
  }
  let html = '<table class="w-full border-collapse">';
  html += '<thead class="sticky top-0 bg-ox-surface z-10"><tr class="text-ox-dim text-[10px]">';
  html += '<th class="text-left px-3 py-1 border-b border-ox-border w-[110px]">Namespace</th>';
  html += '<th class="text-left px-3 py-1 border-b border-ox-border">Term</th>';
  html += '<th class="text-left px-3 py-1 border-b border-ox-border">Target</th>';
  html += '<th class="text-left px-3 py-1 border-b border-ox-border">Value</th>';
  html += '<th class="text-left px-3 py-1 border-b border-ox-border w-[90px]">Qualifier</th>';
  html += '</tr></thead><tbody>';
  for (const a of filtered) {
    const value = a.value === null || a.value === undefined ? '' : String(a.value);
    const qualifier = a.qualifier || '';
    html += '<tr class="border-b border-ox-border/30 hover:bg-ox-hover/40">';
    html += `<td class="px-3 py-0.5 text-ox-blue">${escapeHtml(a.namespace)}</td>`;
    html += `<td class="px-3 py-0.5 text-ox-text">${escapeHtml(a.term)}</td>`;
    html += `<td class="px-3 py-0.5 text-ox-muted">${escapeHtml(a.target)}</td>`;
    html += `<td class="px-3 py-0.5 text-ox-text">${escapeHtml(value) || '<span class="text-ox-dim">—</span>'}</td>`;
    html += `<td class="px-3 py-0.5 text-ox-muted">${escapeHtml(qualifier)}</td>`;
    html += '</tr>';
  }
  html += '</tbody></table>';
  results.innerHTML = html;
}
