// ── Results renderer + nested data + copy + filter tooltip ──
//
// Three concerns folded together because they share `extractRows`,
// the per-tab `state.expandedDataStore`, and the on-screen results
// table:
//   - extractRows + renderResults + renderJson — main view of the run.
//   - showNestedData / showNestedPanel — the floating panel that
//     materialises when the user clicks an array / object cell.
//   - copyColumnValues / copyRowAsJson / copyODataUrl — the in-grid
//     copy actions.
//   - showFilterTooltip / hideFilterTooltip / applyFilterFromTooltip
//     — click-a-cell to filter on its value.
//
// Closes the valueList -> app.js circular (extractRows lives here now)
// and the executor -> app.js circular (renderJson + renderResults +
// extractRows). All imports flow downward — no app.js circular.

import { state } from './state.js';
import { safeHtml, raw } from './html.js';
import { formatDisplayValue, criticalityDot } from './format.js';
import { setStatus } from './status.js';
import { getActiveTab } from './tabs.js';
import {
  showStatsBar,
  hideStatsBar,
  executeQuery,
  buildODataUrl,
} from './executor.js';
import { copyToClipboard } from './clipboard.js';

const COPY_ICON_HTML = '<svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>';

export function extractRows(data) {
  if (data.d) {
    if (data.d.results) return data.d.results;
    return [data.d];
  }
  if (data.value) return data.value;
  return null;
}

export function renderResults(data, elapsedMs, params) {
  const rows = extractRows(data);
  if (!rows || rows.length === 0) {
    state.expandedDataStore = {};
    state.lastResultRows = [];
    document.getElementById('resultsArea').innerHTML =
      '<div class="p-4 text-ox-dim text-sm">No results</div>';
    setStatus('No results');
    hideStatsBar();
    return;
  }

  state.expandedDataStore = {};
  state.lastResultRows = rows;
  const first = rows[0];

  const scalarCols = [];
  const nestedCols = [];
  for (const k of Object.keys(first)) {
    if (k.startsWith('@') || k === '__metadata') continue;
    const val = first[k];
    if (val !== null && typeof val === 'object') {
      nestedCols.push(k);
    } else {
      scalarCols.push(k);
    }
  }

  // SAP-View-driven reshaping: column order from UI.LineItem, plus a
  // propByName lookup used below to render cells per UI.TextArrangement
  // and to hide text-companion columns when they're already folded
  // into their ID column.
  const tab = getActiveTab();
  const info = tab && tab._lastDescribeInfo;
  const sapShape = state.sapViewEnabled && info && Array.isArray(info.properties);
  const propByName = sapShape ? new Map(info.properties.map(p => [p.name, p])) : null;
  // Text-companion columns that will be folded into their ID column's
  // cell — we skip rendering them as standalone columns when the
  // arrangement is anything other than TextSeparate.
  const foldedTextCols = new Set();
  if (sapShape) {
    for (const p of info.properties) {
      if (!p.text_path) continue;
      const arrangement = p.text_arrangement || 'textfirst';
      if (arrangement !== 'textseparate') foldedTextCols.add(p.text_path);
    }
  }
  // Reorder scalars: declared LineItem columns first in position order,
  // then whatever else the response carried (keys, RequestAtLeast
  // fields, technical columns). Only applies in SAP View.
  let orderedScalars = scalarCols;
  if (sapShape && Array.isArray(info.line_item) && info.line_item.length > 0) {
    const scalarSet = new Set(scalarCols);
    const seen = new Set();
    const head = [];
    for (const li of info.line_item) {
      const name = li && li.value_path;
      if (name && scalarSet.has(name) && !seen.has(name)) {
        head.push(name);
        seen.add(name);
      }
    }
    const tail = scalarCols.filter(c => !seen.has(c));
    orderedScalars = [...head, ...tail];
  }
  if (sapShape) {
    orderedScalars = orderedScalars.filter(c => !foldedTextCols.has(c));
  }

  const allCols = [...orderedScalars, ...nestedCols];

  // Estimate JSON size for stats
  const jsonSize = new Blob([JSON.stringify(data)]).size;
  showStatsBar(rows.length, jsonSize, elapsedMs || 0);

  const headerCells = allCols.map(col => {
    const isNested = nestedCols.includes(col);
    const label = isNested ? `${col} ↗` : col;
    const copyButton = isNested
      ? ''
      : safeHtml`
        <button class="copy-btn" data-action="copy-col" data-col="${col}" title="Copy column values">
          ${raw(COPY_ICON_HTML)}
        </button>`;
    return safeHtml`
      <th class="text-left px-3 py-1.5 bg-ox-panel text-ox-dim border-b border-ox-border font-medium sticky top-0 group">
        <span class="mr-1">${label}</span>
        ${raw(copyButton)}
      </th>`;
  }).join('');

  const rowCells = rows.map((row, i) => {
    const stripe = i % 2 === 0 ? '' : 'bg-ox-surface/50';
    const cells = allCols.map(col => {
      const val = row[col];
      if (val === null || val === undefined) {
        return '<td class="px-3 py-1 text-ox-dim">—</td>';
      }
      if (Array.isArray(val)) {
        const storeKey = `r${i}_${col}`;
        state.expandedDataStore[storeKey] = val;
        const count = val.length;
        const suffix = count !== 1 ? 's' : '';
        return safeHtml`
          <td class="px-3 py-1">
            <span class="expand-badge text-[10px] px-1.5 py-0.5 rounded-sm font-mono inline-block" data-action="nested" data-key="${storeKey}" data-col="${col}">${count} item${suffix}</span>
          </td>`;
      }
      if (typeof val === 'object') {
        const storeKey = `r${i}_${col}`;
        state.expandedDataStore[storeKey] = val;
        return safeHtml`
          <td class="px-3 py-1">
            <span class="expand-badge text-[10px] px-1.5 py-0.5 rounded-sm font-mono inline-block" data-action="nested" data-key="${storeKey}" data-col="${col}">object</span>
          </td>`;
      }

      const text = String(val);
      // SAP View: apply sap:display-format to the raw value first
      // (Date strips the time, UpperCase upcases, ...), THEN compose
      // with Common.Text per UI.TextArrangement. The raw `text` is
      // still what goes into filter-tooltip data attributes so
      // click-to-filter uses the unmodified key.
      const prop = sapShape ? propByName.get(col) : null;
      const formatted = prop && prop.display_format
        ? formatDisplayValue(text, prop.display_format, prop.edm_type)
        : text;
      let display = formatted;
      if (sapShape && prop && prop.text_path) {
        const companion = row[prop.text_path];
        const companionText = companion === null || companion === undefined
          ? ''
          : String(companion);
        const arr = prop.text_arrangement || 'textfirst';
        if (arr === 'textfirst' && companionText) {
          display = `${companionText} (${formatted})`;
        } else if (arr === 'textlast' && companionText) {
          display = `${formatted} (${companionText})`;
        } else if (arr === 'textonly') {
          display = companionText || formatted;
        }
      }
      const critDot = sapShape ? criticalityDot(prop, row) : '';
      return safeHtml`
        <td class="px-3 py-1 text-ox-text whitespace-nowrap cursor-pointer" data-action="cell-click" data-cell-col="${col}" data-cell-val="${text}">
          ${raw(critDot)}${display}
        </td>`;
    }).join('');

    // Row copy button (Feature 5)
    const storeKey = `row_${i}`;
    state.expandedDataStore[storeKey] = row;
    const copyCell = safeHtml`
      <td class="px-2 py-1">
        <button class="copy-btn row-copy-btn" data-action="copy-row" data-key="${storeKey}" title="Copy row as JSON">
          ${raw(COPY_ICON_HTML)}
        </button>
      </td>`;

    return safeHtml`
      <tr class="hover:bg-ox-amberGlow border-b border-ox-border/30 transition-colors ${stripe}" data-row-idx="${i}">
        ${raw(cells)}${raw(copyCell)}
      </tr>`;
  }).join('');

  const html = safeHtml`
    <div class="overflow-auto h-full">
      <table class="w-full text-xs font-mono border-collapse">
        <thead>
          <tr>
            ${raw(headerCells)}
            <th class="text-left px-2 py-1.5 bg-ox-panel border-b border-ox-border sticky top-0 w-6"></th>
          </tr>
        </thead>
        <tbody>${raw(rowCells)}</tbody>
      </table>
    </div>`;

  document.getElementById('resultsArea').innerHTML = html;

  // state.lastResultRows already set above for copy operations

  setStatus(`${rows.length} row(s)${nestedCols.length ? ' — click badges to view expanded data' : ''}`);
}

export function showNestedData(storeKey, colName) {
  const data = state.expandedDataStore[storeKey];
  if (!data) return;

  const rows = Array.isArray(data) ? data : [data];
  if (rows.length === 0) { alert('No nested data'); return; }

  const first = rows[0];
  if (typeof first !== 'object' || first === null) {
    const json = JSON.stringify(data, null, 2);
    showNestedPanel(colName, safeHtml`<pre class="text-xs font-mono text-ox-text p-3 whitespace-pre">${json}</pre>`);
    return;
  }

  const cols = Object.keys(first).filter(k => !k.startsWith('@') && k !== '__metadata');
  const headerCells = cols
    .map(c => safeHtml`<th class="text-left px-2 py-1 bg-ox-panel text-ox-dim border-b border-ox-border font-medium sticky top-0">${c}</th>`)
    .join('');
  const bodyRows = rows
    .map(row => {
      const cells = cols
        .map(c => {
          const v = row[c];
          const t = (v === null || v === undefined) ? '' : (typeof v === 'object' ? JSON.stringify(v) : String(v));
          return safeHtml`<td class="px-2 py-0.5 text-ox-text whitespace-nowrap">${t}</td>`;
        })
        .join('');
      return safeHtml`<tr class="border-b border-ox-border/30">${raw(cells)}</tr>`;
    })
    .join('');
  const html = safeHtml`
    <div class="overflow-auto max-h-64">
      <table class="w-full text-xs font-mono border-collapse">
        <thead><tr>${raw(headerCells)}</tr></thead>
        <tbody>${raw(bodyRows)}</tbody>
      </table>
    </div>`;
  showNestedPanel(colName, html);
}

function showNestedPanel(title, contentHtml) {
  const existing = document.getElementById('nestedPanel');
  if (existing) existing.remove();

  const panel = document.createElement('div');
  panel.id = 'nestedPanel';
  panel.className = 'fixed bottom-8 right-4 w-[600px] bg-ox-panel border border-ox-border rounded-lg shadow-2xl z-40';
  panel.style.animation = 'slideUp 0.2s ease';
  // contentHtml is built by callers (showNestedData, etc.) using safeHtml
  // for every interpolation, so we pass it through unescaped via raw().
  panel.innerHTML = safeHtml`
    <div class="px-4 py-2 border-b border-ox-border flex items-center justify-between">
      <div class="flex items-center gap-2">
        <div class="w-1.5 h-1.5 rounded-full bg-ox-blue"></div>
        <span class="text-xs font-mono font-medium text-ox-text">${title}</span>
      </div>
      <button data-action="close-nested" class="text-ox-dim hover:text-ox-text text-xs px-2 py-0.5 rounded-sm hover:bg-ox-hover">close</button>
    </div>
    <div class="p-2">${raw(contentHtml)}</div>
  `;
  document.body.appendChild(panel);
}

export function renderJson(data) {
  state.expandedDataStore = {};
  state.lastResultRows = null;
  hideStatsBar();
  const json = JSON.stringify(data, null, 2);
  document.getElementById('resultsArea').innerHTML =
    safeHtml`<pre class="text-xs font-mono text-ox-text p-4 overflow-auto h-full whitespace-pre leading-relaxed">${json}</pre>`;
  setStatus('JSON');
}

// ══════════════════════════════════════════════════════════════
// ── COPY ACTIONS ──
// ══════════════════════════════════════════════════════════════

export function copyColumnValues(colName) {
  const rows = state.lastResultRows || [];
  const values = rows.map(r => {
    const v = r[colName];
    return (v === null || v === undefined) ? '' : String(v);
  });
  copyToClipboard(values.join('\n'), `column "${colName}"`);
}

export function copyRowAsJson(storeKey) {
  const row = state.expandedDataStore[storeKey];
  if (!row) return;
  const json = JSON.stringify(row, null, 2);
  copyToClipboard(json, 'row as JSON');
}

export function copyODataUrl() {
  const params = {
    entity_set: state.currentEntitySet,
    select:  document.getElementById('qSelect').value  || null,
    filter:  document.getElementById('qFilter').value  || null,
    expand:  document.getElementById('qExpand').value  || null,
    orderby: document.getElementById('qOrderby').value || null,
    top:     parseInt(document.getElementById('qTop').value)  || null,
    skip:    parseInt(document.getElementById('qSkip').value) || null,
  };
  const url = buildODataUrl(params);
  if (url) copyToClipboard(url, 'OData URL');
  else setStatus('No service/entity selected');
}

// ══════════════════════════════════════════════════════════════
// ── FILTER TOOLTIP ──
// ══════════════════════════════════════════════════════════════

let filterTooltipTimeout = null;

export function showFilterTooltip(col, val, x, y) {
  clearTimeout(filterTooltipTimeout);
  const tt = document.getElementById('filterTooltip');
  const escapedVal = val.replace(/'/g, "''"); // OData escapes single quotes by doubling
  tt.textContent = `Filter: ${col} eq '${val}'`;
  tt.dataset.col = col;
  tt.dataset.val = escapedVal;
  tt.style.left = `${x}px`;
  tt.style.top  = `${y + 8}px`;
  tt.style.display = 'block';

  filterTooltipTimeout = setTimeout(hideFilterTooltip, 4000);
}

export function hideFilterTooltip() {
  clearTimeout(filterTooltipTimeout);
  document.getElementById('filterTooltip').style.display = 'none';
}

export function applyFilterFromTooltip() {
  const tt = document.getElementById('filterTooltip');
  const col = tt.dataset.col;
  const val = tt.dataset.val;
  if (!col) return;
  const filterVal = `${col} eq '${val}'`;
  document.getElementById('qFilter').value = filterVal;
  hideFilterTooltip();
  // Auto-run
  executeQuery(false);
}
