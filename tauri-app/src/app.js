// ── Entry / wiring module ──
//
// app.js is the entry point: it sets up the profile-select handler,
// wires button clicks + the document-level click delegate, and binds
// keyboard shortcuts. All feature logic lives in dedicated modules;
// this file is intentionally just glue.

import { state } from './state.js';
import { timedInvoke, updateServicePathBar } from './api.js';
import {
  getActiveTab,
  addTab,
  closeTab,
  switchTab,
  saveCurrentTabState,
} from './tabs.js';
import {
  updateProfileAuthUi,
  signOutCurrentProfile,
  removeCurrentProfile,
  signInCurrentProfile,
} from './auth.js';
import { getFavorites, toggleFavorite } from './favorites.js';
import {
  loadProfiles,
  loadService,
  searchServices,
  pickService,
  selectEntity,
  resetResultsArea,
  renderFavoritesOnlySidebar,
} from './services.js';
import {
  showSapViewWarnings,
  renderSelectionFieldsBar,
  openFilterBar,
  closeFilterBar,
  resetFilterBar,
  onFilterBarOpChange,
  applyFilterBar,
} from './query.js';
import {
  renderFioriColsButton,
  renderFioriFilterButton,
  applyFioriFilter,
  applyFioriCols,
} from './fiori.js';
import {
  openValueListPicker,
  selectVariant,
  closeValueListPicker,
  fetchValueListRows,
  pickValueListRow,
} from './valueList.js';
import {
  renderTraceSummary,
  renderTraceInspector,
  renderTraceDetail,
  updateTraceToggleState,
  toggleTraceInspector,
  hideTraceInspector,
  copySelectedTraceAsCurl,
  copySelectedTraceRequestBody,
  copySelectedTraceResponseBody,
} from './trace.js';
import {
  renderDescribe,
  hideDescribe,
  addToSelect,
  addToExpand,
} from './describe.js';
import {
  renderAnnotationBadge,
  openAnnotationInspector,
  closeAnnotationInspector,
  renderAnnotationInspector,
  toggleAnnotationNamespace,
} from './annotations.js';
import { executeQuery } from './executor.js';
import { renderHistoryPanel, replayHistory } from './history.js';
import {
  showNestedData,
  copyColumnValues,
  copyRowAsJson,
  copyODataUrl,
  showFilterTooltip,
  hideFilterTooltip,
  applyFilterFromTooltip,
} from './results.js';
import {
  showAddProfileModal,
  updateAuthModeFields,
  hideAddProfileModal,
  saveProfileModal,
  testProfileModal,
} from './addProfile.js';
import { setStatus } from './status.js';


document.getElementById('profileSelect').addEventListener('change', (e) => {
  const profile = e.target.value || null;

  saveCurrentTabState();
  const tab = getActiveTab();
  if (!tab) return;

  tab.profile = profile;
  tab.servicePath = null;
  tab.serviceVersion = null;
  tab.entitySet = null;
  tab.entitySets = [];
  tab.cachedServices = null;
  tab.lastSearchQuery = null;
  tab._sidebarHtml = undefined;
  tab._sidebarTitle = 'Services';
  tab._sidebarCount = '';
  tab._serviceInput = '';
  tab._queryBarHidden = true;
  tab._describePanelHidden = true;
  tab._statsVisible = false;
  tab._resultsHtml = undefined;
  tab._historyVisible = false;
  tab.httpTraceEntries = [];
  tab.selectedTraceId = null;
  tab._traceVisible = false;
  tab.annotationSummary = null;

  // Sync globals
  state.currentProfile = profile;
  state.currentServicePath = null;
  state.currentEntitySet = null;
  state.entitySets = [];
  state.cachedServices = null;
  state.lastSearchQuery = null;

  document.getElementById('entityList').innerHTML =
    '<div class="px-4 py-8 text-center"><div class="text-ox-dim text-xs">Click Search to browse services</div></div>';
  document.getElementById('queryBar').classList.add('hidden');
  document.getElementById('describePanel').classList.add('hidden');
  document.getElementById('statsBar').classList.add('hidden');
  document.getElementById('historyPanel').classList.add('hidden');
  document.getElementById('traceInspectorPanel').classList.add('hidden');
  updateTraceToggleState(false);
  renderAnnotationBadge(null);
  document.getElementById('serviceInput').value = '';
  document.getElementById('sidebarTitle').textContent = 'Services';
  document.getElementById('sidebarCount').textContent = '';
  updateServicePathBar(null);
  resetResultsArea();
  renderTraceSummary(tab);
  updateProfileAuthUi(profile);

  if (profile) {
    setStatus(`Connected to ${profile}`);
    // If this profile has favorites stored locally, render them immediately
    // from localStorage — no catalog fetch. Search still populates the full list.
    if (getFavorites(profile).length > 0) {
      renderFavoritesOnlySidebar(profile);
    }
  }
});



function updateSapViewToggleState() {
  const btn = document.getElementById('btnSapView');
  const chev = document.getElementById('sapViewChevron');
  if (!btn || !chev) return;
  chev.innerHTML = state.sapViewEnabled ? '&#x25BE;' : '&#x25B4;';
  // Off state = dim amber (primed but inactive). On state = full amber
  // with a subtle glow so it reads clearly as "engaged".
  if (state.sapViewEnabled) {
    btn.classList.add('text-ox-amber', 'border-ox-amber', 'bg-ox-amberGlow');
    btn.classList.remove('text-ox-amberDim', 'border-ox-amberDim/60');
  } else {
    btn.classList.add('text-ox-amberDim', 'border-ox-amberDim/60');
    btn.classList.remove('text-ox-amber', 'border-ox-amber', 'bg-ox-amberGlow');
  }
}

function toggleSapView() {
  state.sapViewEnabled = !state.sapViewEnabled;
  try { localStorage.setItem('ox_sap_view_enabled', state.sapViewEnabled ? '1' : '0'); } catch { /* ignore */ }
  updateSapViewToggleState();
  // Re-render describe panel in place if the active tab has one up.
  // renderDescribe also refreshes the selection-fields chip bar.
  const tab = getActiveTab();
  if (tab && tab._lastDescribeInfo) {
    renderDescribe(tab._lastDescribeInfo);
  } else {
    // No cached describe — still hide any stale chip bar and quick-actions.
    renderSelectionFieldsBar(null);
    renderFioriColsButton(null);
    renderFioriFilterButton(null);
  }
  // Clear any lingering warnings from the previous mode.
  if (!state.sapViewEnabled) showSapViewWarnings([]);
}





// ══════════════════════════════════════════════════════════════
// ── KEYBOARD SHORTCUTS (Feature 8) ──
// ══════════════════════════════════════════════════════════════

document.addEventListener('keydown', (e) => {
  // Escape — close modals / panels
  if (e.key === 'Escape') {
    if (!document.getElementById('addProfileModal').classList.contains('hidden')) {
      hideAddProfileModal();
      return;
    }
    const nested = document.getElementById('nestedPanel');
    if (nested) { nested.remove(); return; }
    hideFilterTooltip();
    return;
  }

  // Ctrl+Enter — run query
  if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
    const active = document.activeElement;
    // Only if focus is somewhere in the query bar or results area
    const inQueryZone =
      active && (
        active.id === 'qSelect' ||
        active.id === 'qFilter' ||
        active.id === 'qExpand' ||
        active.id === 'qOrderby' ||
        active.id === 'qTop' ||
        active.id === 'qSkip'
      );
    if (inQueryZone || !document.getElementById('queryBar').classList.contains('hidden')) {
      e.preventDefault();
      executeQuery(false);
    }
    return;
  }

  // Enter in service input → search
  if (e.key === 'Enter' && document.activeElement === document.getElementById('serviceInput')) {
    loadService();
  }
});

// ══════════════════════════════════════════════════════════════
// ── INIT ──
// ══════════════════════════════════════════════════════════════

document.addEventListener('DOMContentLoaded', () => {
  // Create first tab
  addTab({ title: 'New Tab' });

  loadProfiles();

  // ── Static button wiring ──
  document.getElementById('btnAddProfile').addEventListener('click', showAddProfileModal);
  document.getElementById('btnProfileSignIn').addEventListener('click', signInCurrentProfile);
  document.getElementById('btnProfileSignOut').addEventListener('click', signOutCurrentProfile);
  document.getElementById('btnRemoveProfile').addEventListener('click', removeCurrentProfile);
  document.getElementById('btnSearch').addEventListener('click', loadService);
  document.getElementById('btnCloseDescribe').addEventListener('click', hideDescribe);
  document.getElementById('btnRun').addEventListener('click', () => executeQuery(false));
  document.getElementById('btnJson').addEventListener('click', () => executeQuery(true));
  document.getElementById('btnCopyUrl').addEventListener('click', copyODataUrl);
  document.getElementById('btnTraceToggle').addEventListener('click', toggleTraceInspector);
  document.getElementById('btnTraceClose').addEventListener('click', hideTraceInspector);
  document.getElementById('btnSapView').addEventListener('click', toggleSapView);
  document.getElementById('annotationBadge').addEventListener('click', openAnnotationInspector);
  document.getElementById('annotationBadge').classList.add('cursor-pointer');
  document.getElementById('btnAiClose').addEventListener('click', closeAnnotationInspector);
  document.getElementById('aiSearch').addEventListener('input', renderAnnotationInspector);
  document.getElementById('btnOpenFilterBar').addEventListener('click', openFilterBar);
  document.getElementById('btnFbClose').addEventListener('click', closeFilterBar);
  document.getElementById('btnFbCancel').addEventListener('click', closeFilterBar);
  document.getElementById('btnFbReset').addEventListener('click', resetFilterBar);
  document.getElementById('btnFbApply').addEventListener('click', applyFilterBar);
  // Operator-change + clear-row need delegation since rows are rebuilt
  // on every open.
  document.getElementById('fbRows').addEventListener('change', (e) => {
    const sel = e.target.closest('select[data-fb="op"]');
    if (sel) onFilterBarOpChange(sel);
  });
  document.getElementById('fbRows').addEventListener('keydown', (e) => {
    if (e.key === 'Enter') applyFilterBar();
  });
  document.getElementById('btnFioriCols').addEventListener('click', applyFioriCols);
  document.getElementById('btnFioriFilter').addEventListener('click', applyFioriFilter);
  document.getElementById('btnVlClose').addEventListener('click', closeValueListPicker);
  document.getElementById('btnVlFetch').addEventListener('click', fetchValueListRows);
  document.getElementById('vlFilter').addEventListener('keydown', (e) => {
    if (e.key === 'Enter') fetchValueListRows();
  });
  document.getElementById('vlSearch').addEventListener('keydown', (e) => {
    if (e.key === 'Enter') fetchValueListRows();
  });
  updateSapViewToggleState();
  document.getElementById('btnSapViewWarningsDismiss').addEventListener('click', () => showSapViewWarnings([]));
  document.getElementById('btnHistoryToggle').addEventListener('click', () => {
    const panel = document.getElementById('historyPanel');
    const tab = getActiveTab();
    if (panel.classList.contains('hidden')) {
      renderHistoryPanel(tab);
      panel.classList.remove('hidden');
      if (tab) tab._historyVisible = true;
    } else {
      panel.classList.add('hidden');
      if (tab) tab._historyVisible = false;
    }
  });
  document.getElementById('btnAddTab').addEventListener('click', () => {
    saveCurrentTabState();
    addTab({ title: 'New Tab', profile: state.currentProfile });
  });
  document.getElementById('btnModalClose').addEventListener('click', hideAddProfileModal);
  document.getElementById('btnCancel').addEventListener('click', hideAddProfileModal);
  document.getElementById('btnSave').addEventListener('click', saveProfileModal);
  document.getElementById('btnTest').addEventListener('click', testProfileModal);
  document.getElementById('mpAuthMode').addEventListener('change', updateAuthModeFields);

  // Filter tooltip click → apply filter
  document.getElementById('filterTooltip').addEventListener('click', applyFilterFromTooltip);

  // Hide filter tooltip when clicking elsewhere
  document.addEventListener('click', (e) => {
    if (e.target.id !== 'filterTooltip' && !e.target.closest('#filterTooltip')) {
      hideFilterTooltip();
    }
  });

  // ── Global event delegation ──
  document.addEventListener('click', (e) => {
    const el = e.target.closest('[data-action]');
    if (!el) return;
    const action = el.dataset.action;

    if (action === 'select') {
      addToSelect(el.dataset.field);
    } else if (action === 'expand') {
      addToExpand(el.dataset.field);
    } else if (action === 'nested') {
      showNestedData(el.dataset.key, el.dataset.col);
    } else if (action === 'close-nested') {
      const p = document.getElementById('nestedPanel');
      if (p) p.remove();
    } else if (action === 'switch-tab') {
      // Don't switch if the close button was clicked
      if (e.target.closest('[data-action="close-tab"]')) return;
      saveCurrentTabState();
      switchTab(el.dataset.tabId);
    } else if (action === 'close-tab') {
      e.stopPropagation();
      saveCurrentTabState();
      closeTab(el.dataset.tabId);
    } else if (action === 'copy-col') {
      e.stopPropagation();
      copyColumnValues(el.dataset.col);
    } else if (action === 'copy-row') {
      e.stopPropagation();
      copyRowAsJson(el.dataset.key);
    } else if (action === 'cell-click') {
      const col = el.dataset.cellCol;
      const val = el.dataset.cellVal;
      const rect = el.getBoundingClientRect();
      showFilterTooltip(col, val, rect.left, rect.bottom);
    } else if (action === 'replay-history') {
      replayHistory(parseInt(el.dataset.idx));
    } else if (action === 'clear-history') {
      const tab = getActiveTab();
      if (tab) { tab.queryHistory = []; renderHistoryPanel(tab); }
    } else if (action === 'select-trace') {
      const tab = getActiveTab();
      if (!tab) return;
      tab.selectedTraceId = parseInt(el.dataset.traceId, 10);
      tab._traceBodyExpanded = false;
      renderTraceInspector(tab);
    } else if (action === 'select-trace-subtab') {
      const tab = getActiveTab();
      if (!tab) return;
      tab._traceSubTab = el.dataset.subtab === 'request' ? 'request' : 'response';
      // Collapse the body expansion when switching sub-tabs — fresh
      // context means the user probably wants the short view first.
      tab._traceBodyExpanded = false;
      renderTraceDetail(tab);
    } else if (action === 'toggle-trace-body') {
      const tab = getActiveTab();
      if (!tab) return;
      tab._traceBodyExpanded = !tab._traceBodyExpanded;
      renderTraceDetail(tab);
    } else if (action === 'copy-trace-curl') {
      copySelectedTraceAsCurl();
    } else if (action === 'copy-trace-request-body') {
      copySelectedTraceRequestBody();
    } else if (action === 'copy-trace-response-body') {
      copySelectedTraceResponseBody();
    } else if (action === 'value-list') {
      openValueListPicker(el.dataset.prop);
    } else if (action === 'vl-select-variant') {
      const idx = parseInt(el.dataset.variantIndex, 10);
      if (!isNaN(idx)) selectVariant(idx);
    } else if (action === 'fb-clear-row') {
      const row = el.closest('[data-fb-row]');
      if (!row) return;
      row.querySelectorAll('input[data-fb="value"], input[data-fb="value-high"]').forEach(i => i.value = '');
      const sel = row.querySelector('select[data-fb="op"]');
      if (sel) {
        sel.value = 'eq';
        onFilterBarOpChange(sel);
      }
    } else if (action === 'ai-toggle-ns') {
      const ns = el.dataset.ns;
      if (!ns) return;
      toggleAnnotationNamespace(ns);
    } else if (action === 'vl-pick') {
      pickValueListRow(parseInt(el.dataset.row, 10));
    } else if (action === 'selection-field') {
      const name = el.dataset.name;
      if (!name) return;
      if (e.shiftKey) {
        // Shift-click: append to $select instead of $filter. Dedupe
        // against whatever is already there so repeated clicks don't
        // bloat the list.
        const input = document.getElementById('qSelect');
        const current = (input.value || '').trim();
        const tokens = current ? current.split(',').map(s => s.trim()).filter(Boolean) : [];
        if (!tokens.includes(name)) tokens.push(name);
        input.value = tokens.join(',');
        input.focus();
        return;
      }
      // Plain click: seed $filter with "<name> eq ''" so the user can
      // complete the literal. Preserve anything already there via `and`.
      const input = document.getElementById('qFilter');
      const current = (input.value || '').trim();
      const snippet = `${name} eq ''`;
      input.value = current ? `${current} and ${snippet}` : snippet;
      input.focus();
      // Caret inside the trailing quotes so typing fills the value.
      const caret = input.value.lastIndexOf("''") + 1;
      if (caret > 0) input.setSelectionRange(caret, caret);
    } else if (action === 'back-to-services') {
      document.getElementById('serviceInput').value = '';
      const tab = getActiveTab();
      if (tab) tab._serviceInput = '';
      searchServices(state.lastSearchQuery === '' ? '' : state.lastSearchQuery);
    } else if (action === 'pick-service') {
      try {
        const svc = JSON.parse(el.dataset.svc || '{}');
        pickService(svc);
      } catch { /* ignore parse error */ }
    } else if (action === 'select-entity') {
      document.querySelectorAll('.sidebar-item').forEach(s => s.classList.remove('active'));
      el.classList.add('active');
      selectEntity(el.dataset.entityName, el);
    } else if (action === 'toggle-favorite') {
      e.stopPropagation();
      // Pull the full service object from the parent sidebar item so we store
      // {technical_name, title, version, ...} — not just the name.
      const parent = el.closest('[data-svc]');
      let svc;
      try { svc = parent ? JSON.parse(parent.dataset.svc) : { technical_name: el.dataset.svcName }; }
      catch { svc = { technical_name: el.dataset.svcName }; }
      toggleFavorite(svc, el);
    }
  });
});
