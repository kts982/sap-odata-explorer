// ── Describe panel renderer ──
//
// Renders the entity-set describe view: properties table + nav-properties
// table + the SAP-view layered hints (Common.Text companions, Measures
// units, semantic-key markers, criticality, value-help buttons, Fiori
// readiness panel below).
//
// `addToSelect` / `addToExpand` are click-target helpers used by the
// delegate when a user clicks a row in the describe tables.
//
// All imports flow downward — no circular back to app.js.

import { state } from './state.js';
import { escapeHtml, safeHtml } from './html.js';
import { criticalityHint, valueListHint } from './format.js';
import { getActiveTab } from './tabs.js';
import { propertyFlagHints, renderSelectionFieldsBar } from './query.js';
import {
  renderFioriColsButton,
  renderFioriFilterButton,
  renderFioriReadinessPanel,
} from './fiori.js';

export function renderDescribe(info) {
  const panel = document.getElementById('describePanel');
  panel.classList.remove('hidden');

  // Cache so the SAP-view toggle can re-render without another fetch.
  const tab = getActiveTab();
  if (tab) tab._lastDescribeInfo = info;

  // Selection-fields chip bar above the query inputs (SAP View only).
  renderSelectionFieldsBar(info);
  // UI.LineItem → "Fiori cols" quick-action button next to $select.
  renderFioriColsButton(info);
  // UI.SelectionVariant → "Fiori filter" button next to "Fiori cols".
  renderFioriFilterButton(info);

  // Title: technical name always; in SAP view, append the HeaderInfo
  // singular/plural so the entity reads as "WarehouseOrderType · Warehouse Order / Warehouse Orders".
  const titleEl = document.getElementById('entityTitle');
  titleEl.textContent = info.name;
  if (state.sapViewEnabled && info.header_info) {
    const hi = info.header_info;
    const parts = [];
    if (hi.type_name) parts.push(hi.type_name);
    if (hi.type_name_plural && hi.type_name_plural !== hi.type_name) parts.push(hi.type_name_plural);
    if (parts.length) {
      titleEl.innerHTML = safeHtml`${info.name}<span class="text-ox-dim ml-2">·</span><span class="text-ox-blue ml-2">${parts.join(' / ')}</span>`;
    }
  }

  let html = '<div class="grid grid-cols-1 lg:grid-cols-2 gap-4">';

  // Properties
  html += '<div class="overflow-auto"><table class="w-full text-xs font-mono"><thead><tr class="text-ox-dim">';
  html += '<th class="text-left pb-1.5 bg-ox-surface pr-3">Property</th>';
  html += '<th class="text-left pb-1.5 bg-ox-surface pr-3">Type</th>';
  html += '<th class="text-left pb-1.5 bg-ox-surface pr-3">Key</th>';
  html += '<th class="text-left pb-1.5 bg-ox-surface">Label</th>';
  html += '</tr></thead><tbody>';
  for (const p of info.properties) {
    const keyMark = p.is_key ? '<span class="text-ox-amber">&#9679;</span>' : '';
    // SAP view: surface the text-companion property ("↦ TextProp") next to
    // the property name. The arrow hints that this column has an associated
    // description field that Fiori renders together with it.
    const textHint = state.sapViewEnabled && p.text_path
      ? ` <span class="text-ox-blue text-[10px]" title="Common.Text → ${escapeHtml(p.text_path)}">&#x21A6; ${escapeHtml(p.text_path)}</span>`
      : '';
    const currencyHint = state.sapViewEnabled && p.iso_currency_path
      ? ` <span class="text-ox-green text-[10px]" title="Measures.ISOCurrency → ${escapeHtml(p.iso_currency_path)}">&curren; ${escapeHtml(p.iso_currency_path)}</span>`
      : '';
    const unitHint = state.sapViewEnabled && p.unit_path && !p.iso_currency_path
      ? ` <span class="text-ox-green text-[10px]" title="Measures.Unit / sap:unit → ${escapeHtml(p.unit_path)}">&#8593; ${escapeHtml(p.unit_path)}</span>`
      : '';
    const titleHint = state.sapViewEnabled && info.header_info && info.header_info.title_path === p.name
      ? ' <span class="text-ox-amber text-[10px]" title="Used as UI.HeaderInfo.Title">title</span>'
      : '';
    const semKeyHint = state.sapViewEnabled && Array.isArray(info.semantic_keys) && info.semantic_keys.includes(p.name)
      ? ' <span class="text-ox-amber text-[10px]" title="Common.SemanticKey — business-key property (vs the technical primary key)">biz key</span>'
      : '';
    const flagHints = state.sapViewEnabled ? propertyFlagHints(p) : '';
    const critHint = state.sapViewEnabled ? criticalityHint(p) : '';
    const vlHint = valueListHint(p, state.sapViewEnabled);
    // Dim the row when SAP View is on and Fiori would hide this
    // property (UI.Hidden or FieldControl=Hidden). Row stays visible
    // and clickable — the muted text just makes it visually recede.
    const hiddenRow = state.sapViewEnabled && (p.hidden || (p.field_control && p.field_control.kind === 'hidden'));
    const rowCls = hiddenRow ? 'opacity-60' : '';
    const nameCls = hiddenRow ? 'text-ox-dim' : 'text-ox-text';
    html += `<tr class="hover:bg-ox-amberGlow cursor-pointer transition-colors ${rowCls}" data-action="select" data-field="${escapeHtml(p.name)}">`;
    html += `<td class="py-0.5 pr-3 ${nameCls}">${escapeHtml(p.name)}${textHint}${currencyHint}${unitHint}${titleHint}${semKeyHint}${critHint}${flagHints}${vlHint}</td>`;
    html += `<td class="py-0.5 pr-3 text-ox-dim">${escapeHtml(p.edm_type.replace('Edm.', ''))}</td>`;
    html += `<td class="py-0.5 pr-3 text-center">${keyMark}</td>`;
    html += `<td class="py-0.5 text-ox-muted">${escapeHtml(p.label || '')}</td>`;
    html += '</tr>';
  }
  html += '</tbody></table></div>';

  // Nav properties
  if (info.nav_properties.length > 0) {
    html += '<div class="overflow-auto"><table class="w-full text-xs font-mono"><thead><tr class="text-ox-dim">';
    html += '<th class="text-left pb-1.5 bg-ox-surface pr-3">Navigation</th>';
    html += '<th class="text-left pb-1.5 bg-ox-surface pr-3">Target</th>';
    html += '<th class="text-left pb-1.5 bg-ox-surface">Mult.</th>';
    html += '</tr></thead><tbody>';
    for (const n of info.nav_properties) {
      html += `<tr class="hover:bg-ox-amberGlow cursor-pointer transition-colors" data-action="expand" data-field="${escapeHtml(n.name)}">`;
      html += `<td class="py-0.5 pr-3 text-ox-text">${escapeHtml(n.name)}</td>`;
      html += `<td class="py-0.5 pr-3 text-ox-dim">${escapeHtml(n.target_type)}</td>`;
      html += `<td class="py-0.5 text-ox-muted">${escapeHtml(n.multiplicity)}</td>`;
      html += '</tr>';
    }
    html += '</tbody></table></div>';
  }

  html += '</div>';
  if (state.sapViewEnabled) {
    html += renderFioriReadinessPanel(info);
  }
  document.getElementById('describeContent').innerHTML = html;
}

export function hideDescribe() {
  document.getElementById('describePanel').classList.add('hidden');
}

// Click-to-add helpers used by the document-level delegate when a user
// picks a row in the describe panel: properties append to $select, nav
// properties append to $expand. Dedupes so multi-clicks don't pile up.
export function addToSelect(fieldName) {
  const el = document.getElementById('qSelect');
  const current = el.value.split(',').map(s => s.trim()).filter(Boolean);
  if (!current.includes(fieldName)) {
    current.push(fieldName);
    el.value = current.join(',');
  }
}

export function addToExpand(navName) {
  const el = document.getElementById('qExpand');
  const current = el.value.split(',').map(s => s.trim()).filter(Boolean);
  if (!current.includes(navName)) {
    current.push(navName);
    el.value = current.join(',');
  }
}
