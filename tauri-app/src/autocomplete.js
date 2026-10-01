// ── Query-bar autocomplete ──
//
// Suggests from the described entity while typing in the query bar:
//   $select / $orderby — property names ($orderby: then asc / desc)
//   $expand            — navigation property names
//   $filter            — property names and functions; comparison
//                        operators after a property; and / or after a
//                        value. Nothing inside a string literal.
// The token at the caret is replaced on accept. Arrow keys move, Enter /
// Tab accept, Escape closes; Ctrl+Enter still runs the query. Names and
// labels come from SAP metadata, so every row is built with safeHtml.
//
// The token logic lives in suggest.js (pure, Node-testable).
//
// All imports flow downward — no circular back to app.js.

import { safeHtml } from './html.js';
import { getActiveTab, currentDescribeInfo } from './tabs.js';
import { serviceODataVersion } from './format.js';
import { suggestionsFor } from './suggest.js';

const FIELD_MODES = { qSelect: 'select', qFilter: 'filter', qOrderby: 'orderby', qExpand: 'expand' };

// ── DOM wiring ──

let popup = null;
let state = null; // { input, from, to, items, selected }

function ensurePopup() {
  if (popup) return popup;
  popup = document.createElement('div');
  popup.id = 'acPopup';
  popup.setAttribute('role', 'listbox');
  popup.className = 'fixed z-50 hidden bg-ox-panel border border-ox-border rounded-sm shadow-lg text-[11px] font-mono max-h-64 overflow-auto min-w-[14rem]';
  // Keep focus in the input while clicking a suggestion.
  popup.addEventListener('mousedown', e => e.preventDefault());
  popup.addEventListener('click', e => {
    const row = e.target.closest('[data-ac-idx]');
    if (row) accept(parseInt(row.dataset.acIdx, 10));
  });
  document.body.appendChild(popup);
  return popup;
}

function close() {
  if (popup) popup.classList.add('hidden');
  state = null;
}

function render() {
  const el = ensurePopup();
  el.innerHTML = state.items.map((item, i) => safeHtml`
    <div class="px-2 py-1 cursor-pointer flex gap-3 justify-between ${i === state.selected ? 'bg-ox-hover' : ''}" role="option" data-ac-idx="${i}">
      <span class="text-ox-text">${item.value}</span><span class="text-ox-dim truncate">${item.detail || ''}</span>
    </div>`).join('');
  const rect = state.input.getBoundingClientRect();
  el.style.left = `${rect.left}px`;
  el.style.top = `${rect.bottom + 2}px`;
  el.classList.remove('hidden');
  const sel = el.querySelector(`[data-ac-idx="${state.selected}"]`);
  if (sel) sel.scrollIntoView({ block: 'nearest' });
}

function update(input) {
  const info = currentDescribeInfo(getActiveTab());
  const mode = FIELD_MODES[input.id];
  const caret = input.selectionStart ?? input.value.length;
  const { from, to, items } = suggestionsFor(mode, input.value, caret, info, serviceODataVersion(getActiveTab()));
  // An exact single match adds nothing.
  if (items.length === 0 || (items.length === 1 && items[0].value === input.value.slice(from, to))) {
    close();
    return;
  }
  state = { input, from, to, items, selected: 0 };
  render();
}

function accept(index) {
  if (!state) return;
  const item = state.items[index];
  const { input, from, to } = state;
  if (!item) return;
  input.value = input.value.slice(0, from) + item.insert + input.value.slice(to);
  const caret = from + item.insert.length;
  input.setSelectionRange(caret, caret);
  close();
  input.focus();
  // Chained suggestions (property → operator) read better without a pause.
  if (item.insert.endsWith(' ') || item.insert.endsWith('(')) update(input);
}

export function attachQueryAutocomplete() {
  for (const id of Object.keys(FIELD_MODES)) {
    const input = document.getElementById(id);
    if (!input) continue;
    input.setAttribute('autocomplete', 'off');
    input.addEventListener('input', () => update(input));
    input.addEventListener('blur', () => close());
    input.addEventListener('keydown', e => {
      if (!state || state.input !== input || e.ctrlKey || e.metaKey) return;
      if (e.key === 'ArrowDown') {
        state.selected = (state.selected + 1) % state.items.length;
        render();
        e.preventDefault();
      } else if (e.key === 'ArrowUp') {
        state.selected = (state.selected - 1 + state.items.length) % state.items.length;
        render();
        e.preventDefault();
      } else if (e.key === 'Enter' || e.key === 'Tab') {
        accept(state.selected);
        e.preventDefault();
      } else if (e.key === 'Escape') {
        close();
        e.preventDefault();
        e.stopPropagation();
      }
    });
  }
}
