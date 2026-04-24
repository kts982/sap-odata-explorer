// ── Tauri invoke wrapper ──
const { invoke } = window.__TAURI__.core;

// ══════════════════════════════════════════════════════════════
// ── TAB SYSTEM ──
// Each tab carries its own independent state.
// ══════════════════════════════════════════════════════════════

let tabs = [];         // array of tab objects
let activeTabId = null;
let profileMap = new Map();

function createTab(opts = {}) {
  const id = 'tab_' + Date.now() + '_' + Math.random().toString(36).slice(2);
  return {
    id,
    title: opts.title || 'New Tab',
    // per-tab state
    profile: opts.profile || null,
    servicePath: opts.servicePath || null,
    serviceVersion: opts.serviceVersion || null,   // 'V2' | 'V4' | null
    entitySet: opts.entitySet || null,
    entitySets: [],
    cachedServices: null,
    lastSearchQuery: null,
    // query history (last 20, in-memory)
    queryHistory: [],
    // last query params (for "history" re-use)
    lastParams: null,
    httpTraceEntries: [],
    selectedTraceId: null,
    _traceVisible: false,
    _traceSubTab: 'response',
    annotationSummary: null,
  };
}

function getTab(id) {
  return tabs.find(t => t.id === id) || null;
}

function getActiveTab() {
  return getTab(activeTabId);
}

function addTab(opts = {}) {
  const tab = createTab(opts);
  tabs.push(tab);
  renderTabBar();
  switchTab(tab.id);
  // If there's an active profile, auto-load services (shows favorites at top)
  if (currentProfile && cachedServices) {
    tab.profile = currentProfile;
    tab.cachedServices = cachedServices;
    searchServices('');
  }
  return tab;
}

function closeTab(id) {
  if (tabs.length <= 1) return; // keep at least 1
  const idx = tabs.findIndex(t => t.id === id);
  if (idx === -1) return;
  tabs.splice(idx, 1);
  if (activeTabId === id) {
    const next = tabs[Math.min(idx, tabs.length - 1)];
    renderTabBar();
    switchTab(next.id);
  } else {
    renderTabBar();
  }
}

function switchTab(id) {
  activeTabId = id;
  renderTabBar();
  restoreTabUI();
}

function renderTabBar() {
  const bar = document.getElementById('tabBar');
  const addBtn = document.getElementById('btnAddTab');
  // Remove all tab elements (not the add button)
  [...bar.querySelectorAll('.tab-item')].forEach(el => el.remove());

  for (const tab of tabs) {
    const el = document.createElement('div');
    el.className = 'tab-item' + (tab.id === activeTabId ? ' active' : '');
    el.dataset.tabId = tab.id;

    const titleEl = document.createElement('span');
    titleEl.className = 'tab-title';
    titleEl.textContent = tab.title;

    const closeEl = document.createElement('span');
    closeEl.className = 'tab-close';
    closeEl.textContent = '×';
    closeEl.dataset.action = 'close-tab';
    closeEl.dataset.tabId = tab.id;

    el.appendChild(titleEl);
    if (tabs.length > 1) el.appendChild(closeEl);
    el.dataset.action = 'switch-tab';

    bar.insertBefore(el, addBtn);
  }
}

/** Save current UI state into the active tab, then restore the new tab's UI */
function saveCurrentTabState() {
  const tab = getActiveTab();
  if (!tab) return;
  // Save query bar values
  tab._qSelect  = document.getElementById('qSelect').value;
  tab._qFilter  = document.getElementById('qFilter').value;
  tab._qExpand  = document.getElementById('qExpand').value;
  tab._qOrderby = document.getElementById('qOrderby').value;
  tab._qTop     = document.getElementById('qTop').value;
  tab._qSkip    = document.getElementById('qSkip').value;
  // Save results HTML and stats
  tab._resultsHtml = document.getElementById('resultsArea').innerHTML;
  tab._statsVisible = !document.getElementById('statsBar').classList.contains('hidden');
  tab._statRows  = document.getElementById('statRows').textContent;
  tab._statSize  = document.getElementById('statSize').textContent;
  tab._statTiming = document.getElementById('statTiming').innerHTML;
  tab._describePanelHidden = document.getElementById('describePanel').classList.contains('hidden');
  tab._describeTitle = document.getElementById('entityTitle').textContent;
  tab._describeHtml = document.getElementById('describeContent').innerHTML;
  tab._queryBarHidden = document.getElementById('queryBar').classList.contains('hidden');
  tab._queryEntitySet = document.getElementById('queryEntitySet').textContent;
  tab._historyVisible = !document.getElementById('historyPanel').classList.contains('hidden');
  tab._traceVisible = !document.getElementById('traceInspectorPanel').classList.contains('hidden');
  tab._sidebarTitle = document.getElementById('sidebarTitle').textContent;
  tab._sidebarCount = document.getElementById('sidebarCount').textContent;
  tab._sidebarHtml = document.getElementById('entityList').innerHTML;
  tab._serviceInput = document.getElementById('serviceInput').value;
  // Save backing data for copy/expand
  tab._expandedDataStore = { ...expandedDataStore };
  tab._lastResultRows = lastResultRows;
}

function restoreTabUI() {
  const tab = getActiveTab();
  if (!tab) return;

  // Sync global convenience vars (used in some legacy fn calls)
  currentProfile    = tab.profile;
  currentServicePath = tab.servicePath;
  currentEntitySet  = tab.entitySet;
  entitySets        = tab.entitySets;
  cachedServices    = tab.cachedServices;
  lastSearchQuery   = tab.lastSearchQuery;
  expandedDataStore = tab._expandedDataStore || {};
  lastResultRows = tab._lastResultRows || null;

  // Sync profile dropdown
  document.getElementById('profileSelect').value = currentProfile || '';
  updateProfileAuthUi(currentProfile);

  // Service path bar
  updateServicePathBar(tab);

  // Service input
  document.getElementById('serviceInput').value = tab._serviceInput || '';

  // Sidebar
  document.getElementById('sidebarTitle').textContent = tab._sidebarTitle || 'Services';
  document.getElementById('sidebarCount').textContent = tab._sidebarCount || '';
  if (tab._sidebarHtml !== undefined) {
    document.getElementById('entityList').innerHTML = tab._sidebarHtml;
    // Re-attach sidebar item click handlers (lost when innerHTML was set)
    reattachSidebarHandlers();
  } else {
    document.getElementById('entityList').innerHTML =
      '<div class="px-4 py-8 text-center"><div class="text-ox-dim text-xs">Select a profile and search</div></div>';
  }

  // Describe panel
  if (tab._describePanelHidden === false) {
    document.getElementById('describePanel').classList.remove('hidden');
    document.getElementById('entityTitle').textContent = tab._describeTitle || '';
    document.getElementById('describeContent').innerHTML = tab._describeHtml || '';
  } else {
    document.getElementById('describePanel').classList.add('hidden');
  }

  // Query bar
  if (tab._queryBarHidden === false) {
    document.getElementById('queryBar').classList.remove('hidden');
    document.getElementById('queryEntitySet').textContent = tab._queryEntitySet || '';
    document.getElementById('qSelect').value  = tab._qSelect  || '';
    document.getElementById('qFilter').value  = tab._qFilter  || '';
    document.getElementById('qExpand').value  = tab._qExpand  || '';
    document.getElementById('qOrderby').value = tab._qOrderby || '';
    document.getElementById('qTop').value     = tab._qTop     !== undefined ? tab._qTop : '20';
    document.getElementById('qSkip').value    = tab._qSkip    || '';
  } else {
    document.getElementById('queryBar').classList.add('hidden');
    document.getElementById('qSelect').value  = '';
    document.getElementById('qFilter').value  = '';
    document.getElementById('qExpand').value  = '';
    document.getElementById('qOrderby').value = '';
    document.getElementById('qTop').value     = '20';
    document.getElementById('qSkip').value    = '';
  }

  // History panel
  if (tab._historyVisible) {
    renderHistoryPanel(tab);
    document.getElementById('historyPanel').classList.remove('hidden');
  } else {
    document.getElementById('historyPanel').classList.add('hidden');
  }

  renderTraceSummary(tab);
  if (tab._traceVisible) {
    renderTraceInspector(tab);
    document.getElementById('traceInspectorPanel').classList.remove('hidden');
  } else {
    document.getElementById('traceInspectorPanel').classList.add('hidden');
  }
  updateTraceToggleState(tab._traceVisible);

  renderAnnotationBadge(tab.annotationSummary);

  // Stats bar
  if (tab._statsVisible) {
    document.getElementById('statRows').textContent = tab._statRows || '';
    document.getElementById('statSize').textContent = tab._statSize || '';
    document.getElementById('statTiming').innerHTML = tab._statTiming || '';
    document.getElementById('statsBar').classList.remove('hidden');
  } else {
    document.getElementById('statsBar').classList.add('hidden');
  }

  // Results
  if (tab._resultsHtml !== undefined) {
    document.getElementById('resultsArea').innerHTML = tab._resultsHtml;
  } else {
    resetResultsArea();
  }
}

/** Re-attach event handlers on sidebar items after innerHTML restore */
function reattachSidebarHandlers() {
  // All sidebar items (back link, service items, star buttons, entity items)
  // are handled by document-level delegation only — nothing to re-attach.
}

// ══════════════════════════════════════════════════════════════
// ── GLOBAL CONVENIENCE STATE (mirrors active tab) ──
// ══════════════════════════════════════════════════════════════

let currentProfile    = null;
let currentServicePath = null;
let currentEntitySet  = null;
let entitySets        = [];
let cachedServices    = null;
let lastSearchQuery   = null;
let expandedDataStore = {};
let lastResultRows    = null; // raw result rows for copy operations

// Global SAP-view preference: when enabled, describe panel and results
// surface typed annotation hints (HeaderInfo, Common.Text, etc.) on top
// of the raw EDM data. Persisted so the preference survives app restarts.
let sapViewEnabled = false;
try { sapViewEnabled = localStorage.getItem('ox_sap_view_enabled') === '1'; } catch { /* ignore */ }

function getProfileMeta(profileName) {
  return profileName ? (profileMap.get(profileName) || null) : null;
}

function isBrowserAuthProfile(profileName = currentProfile) {
  return getProfileMeta(profileName)?.auth_mode === 'browser';
}

function updateProfileAuthUi(profileName = currentProfile) {
  const signInBtn  = document.getElementById('btnProfileSignIn');
  const signOutBtn = document.getElementById('btnProfileSignOut');
  const removeBtn  = document.getElementById('btnRemoveProfile');

  // Sign In / Sign Out: only for browser SSO profiles
  if (!profileName || !isBrowserAuthProfile(profileName)) {
    signInBtn.classList.add('hidden');
    signOutBtn.classList.add('hidden');
  } else {
    signInBtn.classList.remove('hidden');
    signOutBtn.classList.remove('hidden');
  }

  // Remove button: shown whenever any profile is selected
  if (!profileName) {
    removeBtn.classList.add('hidden');
  } else {
    removeBtn.classList.remove('hidden');
  }
}

async function signOutCurrentProfile() {
  if (!currentProfile) { setStatus('Select a profile first'); return; }
  if (!isBrowserAuthProfile(currentProfile)) {
    setStatus('Sign Out only applies to browser SSO profiles');
    return;
  }
  try {
    const msg = await invoke('sign_out_profile', { profileName: currentProfile });
    clearTraceState(getActiveTab());
    setStatus(msg);
  } catch (e) {
    setStatus('Sign out failed: ' + e);
  }
}

async function removeCurrentProfile() {
  if (!currentProfile) {
    setStatus('Select a profile first');
    return;
  }
  const name = currentProfile;
  if (!confirm(`Remove profile '${name}'?\n\nThis will also delete its password from the OS keyring.`)) {
    return;
  }
  try {
    const msg = await invoke('remove_profile', { name });
    setStatus(msg);
    clearTraceState(getActiveTab());
    // Reset UI state
    currentProfile = null;
    cachedServices = null;
    document.getElementById('profileSelect').value = '';
    document.getElementById('entityList').innerHTML =
      '<div class="px-4 py-8 text-center"><div class="text-ox-dim text-xs">Select a profile</div></div>';
    updateProfileAuthUi(null);
    updateServicePathBar(null);
    resetResultsArea();
    // Refresh dropdown
    await loadProfiles();
  } catch (e) {
    setStatus('Remove failed: ' + e);
  }
}

function browserAuthMessage(err) {
  return `${String(err)}\n\nBrowser SSO session required. Use the Sign In button next to the profile selector.`;
}

async function signInCurrentProfile() {
  if (!currentProfile) {
    setStatus('Select a profile first');
    return;
  }
  if (!isBrowserAuthProfile(currentProfile)) {
    setStatus('The selected profile does not use browser SSO');
    return;
  }

  setStatus(`Signing in to ${currentProfile}...`);
  try {
    const msg = await timedInvoke('browser_sign_in_profile', { profileName: currentProfile });
    setStatus(msg);
  } catch (e) {
    setStatus('Sign-in failed: ' + e);
    document.getElementById('resultsArea').innerHTML =
      `<div class="p-4 text-ox-red text-sm">${escapeHtml(String(e))}</div>`;
  }
}

// ══════════════════════════════════════════════════════════════
// ── STATUS ──
// ══════════════════════════════════════════════════════════════

function setStatus(msg) {
  document.getElementById('statusText').textContent = msg;
}

function setTime(ms) {
  document.getElementById('statusTime').textContent = ms ? `${ms}ms` : '';
}

function showSpinner() {
  document.getElementById('globalSpinner').classList.remove('hidden');
}

function hideSpinner() {
  document.getElementById('globalSpinner').classList.add('hidden');
}

async function timedInvoke(cmd, args) {
  showSpinner();
  const start = performance.now();
  const originTabId = activeTabId;
  try {
    const result = await invoke(cmd, args);
    setTime(Math.round(performance.now() - start));
    // Commands that touch the network return { data, trace }. Legacy commands
    // still return their value directly.
    if (result && typeof result === 'object' && 'data' in result && Array.isArray(result.trace)) {
      applyTraceToTab(originTabId, result.trace);
      return result.data;
    }
    return result;
  } catch (err) {
    // Network commands serialize errors as { message, trace } — apply the trace
    // and re-throw the plain message so callers keep the string-based API.
    if (err && typeof err === 'object' && 'message' in err && Array.isArray(err.trace)) {
      applyTraceToTab(originTabId, err.trace);
      throw err.message;
    }
    throw err;
  } finally {
    hideSpinner();
  }
}

function applyTraceToTab(tabId, trace) {
  const tab = getTab(tabId);
  if (!tab) return;
  tab.httpTraceEntries = Array.isArray(trace) ? trace : [];
  if (!tab.httpTraceEntries.some(entry => entry.id === tab.selectedTraceId)) {
    tab.selectedTraceId = null;
  }
  ensureTraceSelection(tab);
  if (tab.id === activeTabId) {
    renderTraceSummary(tab);
    if (tab._traceVisible) {
      renderTraceInspector(tab);
    }
  }
}

// ══════════════════════════════════════════════════════════════
// ── SERVICE PATH BAR (Feature 6) ──
// ══════════════════════════════════════════════════════════════

function updateServicePathBar(tab) {
  const bar = document.getElementById('servicePathBar');
  if (tab && tab.servicePath) {
    document.getElementById('servicePathText').textContent = tab.servicePath;
    const verEl = document.getElementById('servicePathVersion');
    if (tab.serviceVersion) {
      verEl.textContent = tab.serviceVersion;
      verEl.className = 'text-[10px] px-1 py-px rounded-sm font-mono ' +
        (tab.serviceVersion === 'V4' ? 'badge-v4' : 'badge-v2');
      verEl.style.display = '';
    } else {
      verEl.style.display = 'none';
    }
    bar.classList.add('visible');
  } else {
    bar.classList.remove('visible');
  }
}

// ══════════════════════════════════════════════════════════════
// ── FAVORITES (Feature 2) ──
// ══════════════════════════════════════════════════════════════

function favKey(profileName) {
  return `ox_favorites_${profileName}`;
}

// Favorites used to be an array of technical_name strings. They are now full
// service objects { technical_name, title, description, service_url, version }.
// Any legacy string entry is normalized to a stub here; once the user re-stars
// or the catalog is fetched, it gets upgraded to the full shape on save.
function getFavorites(profileName) {
  let raw;
  try {
    raw = JSON.parse(localStorage.getItem(favKey(profileName)) || '[]');
  } catch { return []; }
  if (!Array.isArray(raw)) return [];
  return raw.map(entry => {
    if (typeof entry === 'string') {
      return { technical_name: entry, title: entry, description: '', service_url: '', version: '' };
    }
    return entry;
  });
}

function saveFavorites(profileName, list) {
  localStorage.setItem(favKey(profileName), JSON.stringify(list));
}

function favIndex(favs, svcName) {
  return favs.findIndex(f => f.technical_name === svcName);
}

function isFavorite(profileName, svcName) {
  return favIndex(getFavorites(profileName), svcName) !== -1;
}

function toggleFavorite(svc, starEl) {
  const tab = getActiveTab();
  const profile = tab ? tab.profile : currentProfile;
  if (!profile) return;
  const favs = getFavorites(profile);
  const idx = favIndex(favs, svc.technical_name);
  if (idx === -1) {
    favs.push(svc);
    starEl.textContent = '★';
    starEl.classList.add('starred');
  } else {
    favs.splice(idx, 1);
    starEl.textContent = '☆';
    starEl.classList.remove('starred');
  }
  saveFavorites(profile, favs);
  // Re-render the service list to move favorites to top
  const tab2 = getActiveTab();
  if (tab2 && tab2.cachedServices) {
    const filtered = filterServices(tab2.cachedServices, tab2.lastSearchQuery || '');
    renderServiceList(filtered, false);
  } else {
    // No catalog loaded — we're in the favorites-only view, re-render it.
    renderFavoritesOnlySidebar(profile);
  }
}

// ══════════════════════════════════════════════════════════════
// ── PROFILES ──
// ══════════════════════════════════════════════════════════════

async function loadProfiles() {
  try {
    const profiles = await invoke('get_profiles');
    profileMap = new Map(profiles.map(p => [p.name, p]));
    const select = document.getElementById('profileSelect');
    select.innerHTML = '<option value="">Select profile...</option>';
    for (const p of profiles) {
      const opt = document.createElement('option');
      opt.value = p.name;
      opt.textContent = `${p.name} — ${p.base_url.replace('https://', '')}`;
      select.appendChild(opt);
    }
    updateProfileAuthUi(select.value || currentProfile);
  } catch (e) {
    setStatus('Error loading profiles: ' + e);
  }
}

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
  currentProfile = profile;
  currentServicePath = null;
  currentEntitySet = null;
  entitySets = [];
  cachedServices = null;
  lastSearchQuery = null;

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

function resetResultsArea() {
  document.getElementById('resultsArea').innerHTML = `
    <div class="flex items-center justify-center h-full">
      <div class="text-center">
        <div class="text-ox-amber text-3xl mb-3 opacity-20">&#9670;</div>
        <div class="text-ox-dim text-xs leading-relaxed">
          Select a <span class="text-ox-text">profile</span> &middot; search for a <span class="text-ox-text">service</span> &middot; explore <span class="text-ox-text">entities</span>
        </div>
      </div>
    </div>`;
}

// ══════════════════════════════════════════════════════════════
// ── SERVICE SEARCH ──
// ══════════════════════════════════════════════════════════════

// Enter in service input handled by global keydown handler (no duplicate)

async function loadService() {
  const input = document.getElementById('serviceInput').value.trim();
  if (!currentProfile) { setStatus('Select a profile first'); return; }
  // Only treat as a literal path when it starts with `/sap/`. SAP catalog
  // technical names in a customer namespace (e.g. `/NAMESPACE/SERVICE_NAME`)
  // also start with `/` but are NOT service paths — they need catalog
  // resolution like any bare name.
  if (isServicePath(input)) {
    await resolveAndLoadService(input);
  } else {
    await searchServices(input);
  }
}

// True when the given string looks like an SAP OData service path
// (`/sap/opu/odata/...`, `/sap/opu/odata4/...`), not a catalog entry name.
function isServicePath(s) {
  return typeof s === 'string' && s.startsWith('/sap/');
}

async function searchServices(query) {
  if (!currentProfile) return;
  const tab = getActiveTab();

  if (tab && tab.cachedServices && tab.lastSearchQuery === (query || '')) {
    renderServiceList(filterServices(tab.cachedServices, query));
    return;
  }

  setStatus(query ? `Searching '${query}'...` : 'Loading catalog...');

  try {
    if (!cachedServices) {
      cachedServices = await timedInvoke('get_services', {
        profileName: currentProfile,
        search: null,
        v4Only: false,
      });
      if (tab) tab.cachedServices = cachedServices;
    }

    lastSearchQuery = query || '';
    if (tab) tab.lastSearchQuery = lastSearchQuery;

    const filtered = filterServices(cachedServices, query);
    renderServiceList(filtered);
    setStatus(`${filtered.length} service(s)${query ? ` matching '${query}'` : ''}`);
  } catch (e) {
    setStatus('Error: ' + e);
    const message = isBrowserAuthProfile(currentProfile) ? browserAuthMessage(e) : String(e);
    document.getElementById('resultsArea').innerHTML =
      `<div class="p-4 text-ox-red text-sm">${escapeHtml(message)}</div>`;
  }
}

function filterServices(services, query) {
  if (!query) return services;
  const q = query.toLowerCase();
  return services.filter(s =>
    s.technical_name.toLowerCase().includes(q) ||
    s.title.toLowerCase().includes(q) ||
    s.description.toLowerCase().includes(q)
  );
}

function makeSvcItem(svc, starred) {
  const div = document.createElement('div');
  div.className = 'sidebar-item px-3 py-2 cursor-pointer';
  div.dataset.action = 'pick-service';
  div.dataset.svc = JSON.stringify(svc);
  const badgeClass = svc.version === 'V4' ? 'badge-v4' : 'badge-v2';
  div.innerHTML = `
    <div class="flex items-center gap-1.5">
      <span class="text-[9px] px-1 py-px rounded-sm font-mono ${badgeClass}">${escapeHtml(svc.version || '')}</span>
      <span class="text-[13px] text-ox-text truncate font-mono flex-1">${escapeHtml(svc.technical_name)}</span>
      <span class="svc-star${starred ? ' starred' : ''}" data-action="toggle-favorite" data-svc-name="${escapeHtml(svc.technical_name)}">${starred ? '★' : '☆'}</span>
    </div>
    <div class="text-[11px] text-ox-muted truncate mt-0.5 pl-7">${escapeHtml(svc.title || svc.description || '')}</div>
  `;
  return div;
}

// Zero-network sidebar render for when a profile is selected and has favorites
// stored locally. Uses only the data captured in getFavorites — no catalog fetch.
function renderFavoritesOnlySidebar(profile) {
  const favs = getFavorites(profile);
  const list = document.getElementById('entityList');
  document.getElementById('sidebarTitle').textContent = 'Services';
  document.getElementById('sidebarCount').textContent = String(favs.length);
  list.innerHTML = '';
  if (favs.length === 0) return;

  const hdr = document.createElement('div');
  hdr.className = 'px-3 py-1 text-[9px] uppercase tracking-widest text-ox-amber font-medium border-b border-ox-border/40';
  hdr.textContent = 'Favorites';
  list.appendChild(hdr);
  for (const svc of favs) list.appendChild(makeSvcItem(svc, true));

  const footer = document.createElement('div');
  footer.className = 'px-3 py-3 text-[10px] text-ox-dim text-center';
  footer.innerHTML = 'Click <span class="text-ox-text">Search</span> to browse all services';
  list.appendChild(footer);

  const tab = getActiveTab();
  if (tab) {
    tab._sidebarTitle = 'Services';
    tab._sidebarCount = String(favs.length);
    tab._sidebarHtml = list.outerHTML;
  }
}

function renderServiceList(services, saveState = true) {
  const tab = getActiveTab();
  const profile = tab ? tab.profile : currentProfile;

  if (saveState) {
    currentServicePath = null;
    currentEntitySet = null;
    if (tab) { tab.servicePath = null; tab.entitySet = null; }
  }

  document.getElementById('sidebarTitle').textContent = 'Services';
  document.getElementById('sidebarCount').textContent = services.length;

  const list = document.getElementById('entityList');
  list.innerHTML = '';

  if (services.length === 0) {
    list.innerHTML = '<div class="px-4 py-8 text-center"><div class="text-ox-dim text-xs">No services found</div></div>';
    return;
  }

  const favs = profile ? getFavorites(profile) : [];
  const favNames = new Set(favs.map(f => f.technical_name));
  const favorites = services.filter(s => favNames.has(s.technical_name));
  const rest = services.filter(s => !favNames.has(s.technical_name));

  // Upgrade any legacy string-only favorites to full objects now that we have
  // the catalog data. Idempotent: re-saving stable objects is a no-op for the UI.
  if (profile && favorites.length > 0) {
    const byName = new Map(favorites.map(s => [s.technical_name, s]));
    const upgraded = favs.map(f => byName.get(f.technical_name) || f);
    saveFavorites(profile, upgraded);
  }

  if (favorites.length > 0) {
    const hdr = document.createElement('div');
    hdr.className = 'px-3 py-1 text-[9px] uppercase tracking-widest text-ox-amber font-medium border-b border-ox-border/40';
    hdr.textContent = 'Favorites';
    list.appendChild(hdr);
    for (const svc of favorites) list.appendChild(makeSvcItem(svc, true));

    const hdr2 = document.createElement('div');
    hdr2.className = 'px-3 py-1 text-[9px] uppercase tracking-widest text-ox-dim font-medium border-b border-ox-border/40 mt-1';
    hdr2.textContent = 'All Services';
    list.appendChild(hdr2);
  }
  for (const svc of rest) list.appendChild(makeSvcItem(svc, false));

  if (saveState) {
    document.getElementById('queryBar').classList.add('hidden');
    document.getElementById('describePanel').classList.add('hidden');
    document.getElementById('statsBar').classList.add('hidden');
    document.getElementById('historyPanel').classList.add('hidden');
    resetResultsArea();
  }

  // Persist sidebar HTML to tab
  if (tab) {
    tab._sidebarTitle = 'Services';
    tab._sidebarCount = String(services.length);
    tab._sidebarHtml = list.outerHTML;
  }
}

async function pickService(svc) {
  document.getElementById('serviceInput').value = svc.technical_name;
  const tab = getActiveTab();
  if (tab) tab._serviceInput = svc.technical_name;

  if (svc.service_url) {
    let path = svc.service_url;
    if (path.startsWith('http://') || path.startsWith('https://')) {
      try { path = new URL(path).pathname; } catch { /* use as-is */ }
    }
    await resolveAndLoadService(path, svc.version);
  } else {
    await resolveAndLoadService(svc.technical_name, svc.version);
  }
}

async function resolveAndLoadService(input, versionHint) {
  if (!currentProfile) return;
  setStatus(`Resolving '${input}'...`);

  try {
    let path;
    if (isServicePath(input)) {
      path = input;
    } else {
      path = await timedInvoke('resolve_service', {
        profileName: currentProfile,
        service: input,
      });
    }

    currentServicePath = path;
    const tab = getActiveTab();
    if (tab) {
      tab.servicePath = path;
      tab.serviceVersion = versionHint || null;
      tab.title = path.split('/').filter(Boolean).pop() || path;
      renderTabBar();
    }
    updateServicePathBar(tab);

    setStatus(`Loading entities...`);
    const response = await timedInvoke('get_entities', {
      profileName: currentProfile,
      servicePath: currentServicePath,
    });

    const entities = response.entity_sets || [];
    const summary = response.annotation_summary || { total: 0, by_namespace: {} };

    entitySets = entities;
    if (tab) {
      tab.entitySets = entities;
      tab.annotationSummary = summary;
    }
    renderEntityList(entities);
    renderAnnotationBadge(summary);
    setStatus(`${entities.length} entity set(s)`);
    resetResultsArea();
    if (tab) tab._resultsHtml = undefined;
  } catch (e) {
    setStatus('Error: ' + e);
    const message = isBrowserAuthProfile(currentProfile) ? browserAuthMessage(e) : String(e);
    document.getElementById('resultsArea').innerHTML =
      `<div class="p-4 text-ox-red text-sm">${escapeHtml(message)}</div>`;
  }
}

function renderEntityList(entities) {
  const list = document.getElementById('entityList');
  list.innerHTML = '';
  const tab = getActiveTab();

  document.getElementById('sidebarTitle').textContent = 'Entities';
  document.getElementById('sidebarCount').textContent = entities.length;

  // Back link
  const back = document.createElement('div');
  back.className = 'px-3 py-1.5 cursor-pointer text-[11px] text-ox-amber hover:text-ox-text border-b border-ox-border/50 transition-colors';
  back.innerHTML = '&larr; back to services';
  back.dataset.action = 'back-to-services';
  list.appendChild(back);

  for (const es of entities) {
    const div = document.createElement('div');
    div.className = 'sidebar-item px-3 py-2 cursor-pointer';
    div.dataset.action = 'select-entity';
    div.dataset.entityName = es.name;
    div.innerHTML = `
      <div class="text-[13px] text-ox-text font-mono">${escapeHtml(es.name)}</div>
      <div class="text-[10px] text-ox-dim font-mono mt-0.5">${es.keys.join(', ')}</div>
    `;
    // Click handled by document-level delegation (data-action="select-entity")
    list.appendChild(div);
  }

  if (tab) {
    tab._sidebarTitle = 'Entities';
    tab._sidebarCount = String(entities.length);
    tab._sidebarHtml = list.outerHTML;
  }
}

// ══════════════════════════════════════════════════════════════
// ── ENTITY SELECTION ──
// ══════════════════════════════════════════════════════════════

async function selectEntity(entitySetName, element) {
  document.querySelectorAll('.sidebar-item').forEach(el => el.classList.remove('active'));
  if (element) element.classList.add('active');

  currentEntitySet = entitySetName;
  const tab = getActiveTab();
  if (tab) {
    tab.entitySet = entitySetName;
    tab.title = entitySetName;
    renderTabBar();
  }

  document.getElementById('queryBar').classList.remove('hidden');
  document.getElementById('queryEntitySet').textContent = entitySetName;

  document.getElementById('qSelect').value = '';
  document.getElementById('qFilter').value = '';
  document.getElementById('qExpand').value = '';
  document.getElementById('qOrderby').value = '';
  document.getElementById('qTop').value = '20';
  document.getElementById('qSkip').value = '';
  document.getElementById('statsBar').classList.add('hidden');
  document.getElementById('historyPanel').classList.add('hidden');

  setStatus(`Describing ${entitySetName}...`);
  try {
    const info = await timedInvoke('describe_entity', {
      profileName: currentProfile,
      servicePath: currentServicePath,
      entitySet: entitySetName,
    });
    renderDescribe(info);
    setStatus(`${entitySetName} — ${info.properties.length} props, ${info.nav_properties.length} navs`);
  } catch (e) {
    setStatus('Error: ' + e);
  }
}

// ══════════════════════════════════════════════════════════════
// ── DESCRIBE PANEL ──
// ══════════════════════════════════════════════════════════════

// SAP view helper: render small pills for property-level restrictions.
// We only surface *deviations from the default* — filterable/sortable/
// creatable/updatable are normally true, so showing "no filter", "no sort",
// "read-only" is what's informative. required_in_filter=true is also
// visible because it constrains how the user writes $filter.
function propertyFlagHints(p) {
  const badges = [];
  if (p.filterable === false) {
    badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="sap:filterable=false — server rejects $filter on this column">no filter</span>`);
  }
  if (p.sortable === false) {
    badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="sap:sortable=false — server rejects $orderby on this column">no sort</span>`);
  }
  if (p.creatable === false && p.updatable === false) {
    badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="sap:creatable=false and sap:updatable=false — server assigns this value, clients cannot write it">read-only</span>`);
  } else {
    if (p.creatable === false) {
      badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="sap:creatable=false">no create</span>`);
    }
    if (p.updatable === false) {
      badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="sap:updatable=false">no update</span>`);
    }
  }
  if (p.required_in_filter === true) {
    badges.push(`<span class="text-[9px] text-ox-amber bg-ox-amberGlow border border-ox-amber/40 rounded-sm px-1 py-px" title="sap:required-in-filter=true — the server requires $filter to constrain this column">req.filter</span>`);
  }
  return badges.length ? ' ' + badges.join(' ') : '';
}

// Extract identifier-looking tokens from a free-form OData expression.
// Good enough for cross-checking against known property names — any
// identifier that happens to appear in the expression AND matches a
// restricted property name flags as a likely reference.
function extractIdentifiers(text) {
  if (!text) return [];
  return (text.match(/[A-Za-z_][A-Za-z0-9_]*/g) || []);
}

// Pre-flight validator for SAP View. Returns a list of human-readable
// restriction violations — empty list means "OK to run".
function validateQueryRestrictions(params, info) {
  if (!info || !Array.isArray(info.properties)) return [];
  const issues = [];
  const byName = new Map(info.properties.map(p => [p.name, p]));

  if (params.filter) {
    const tokens = new Set(extractIdentifiers(params.filter));
    for (const p of info.properties) {
      if (p.filterable === false && tokens.has(p.name)) {
        issues.push(`'${p.name}' is non-filterable (Capabilities.FilterRestrictions / sap:filterable=false) but referenced in $filter.`);
      }
    }
  }

  if (params.orderby) {
    const tokens = new Set(extractIdentifiers(params.orderby));
    for (const p of info.properties) {
      if (p.sortable === false && tokens.has(p.name)) {
        issues.push(`'${p.name}' is non-sortable but referenced in $orderby.`);
      }
    }
  }

  // required_in_filter: these properties MUST be narrowed in $filter.
  const required = info.properties.filter(p => p.required_in_filter === true);
  if (required.length) {
    const tokens = new Set(extractIdentifiers(params.filter || ''));
    for (const p of required) {
      if (!tokens.has(p.name)) {
        issues.push(`'${p.name}' requires a filter clause (Capabilities.FilterRestrictions.RequiredProperties / sap:required-in-filter).`);
      }
    }
  }

  // byName lookup silences unused warnings; also handy for future checks.
  void byName;
  return issues;
}

// Show or hide the amber warnings strip above the results. An empty
// list hides it. Called by executeQuery and by the SAP-view toggle so
// stale warnings don't linger after the user flips the mode off.
function showSapViewWarnings(issues) {
  const strip = document.getElementById('sapViewWarnings');
  const list = document.getElementById('sapViewWarningsList');
  if (!strip || !list) return;
  if (!issues || issues.length === 0) {
    strip.classList.add('hidden');
    list.textContent = '';
    return;
  }
  strip.classList.remove('hidden');
  list.textContent = issues.map(i => `• ${i}`).join('\n');
}

// Render the "selection fields" chip bar above the query inputs. Only
// visible when SAP View is on, the entity type has UI.SelectionFields,
// and we're looking at that entity's describe panel. Clicking a chip
// seeds $filter with a skeleton clause the user can complete.
function renderSelectionFieldsBar(info) {
  const bar = document.getElementById('selectionFieldsBar');
  const host = document.getElementById('selectionFieldsChips');
  if (!bar || !host) return;
  const fields = sapViewEnabled && info && Array.isArray(info.selection_fields)
    ? info.selection_fields
    : [];
  if (fields.length === 0) {
    bar.classList.add('hidden');
    host.innerHTML = '';
    return;
  }
  bar.classList.remove('hidden');
  // Amber-flag chips whose backing property is required-in-filter, so the
  // user sees at a glance which selection fields the server will reject
  // queries without.
  const byName = new Map(
    Array.isArray(info.properties)
      ? info.properties.map(p => [p.name, p])
      : []
  );
  host.innerHTML = fields
    .map(name => {
      const p = byName.get(name);
      const req = p && p.required_in_filter === true;
      const cls = req
        ? 'text-[10px] px-1.5 py-0.5 rounded-sm text-ox-amber bg-ox-amberGlow border border-ox-amber/40 hover:bg-ox-amber/20'
        : 'btn-ghost text-[10px] px-1.5 py-0.5 rounded-sm';
      const title = req
        ? 'Required in $filter — append and narrow'
        : 'Append to $filter';
      return `<button type="button" class="${cls}" data-action="selection-field" data-name="${escapeHtml(name)}" title="${title}">${escapeHtml(name)}</button>`;
    })
    .join('');
}

// ══════════════════════════════════════════════════════════════
// ── VALUE-LIST PICKER ──
// ══════════════════════════════════════════════════════════════
// State held per-open: the full Property info, the ValueList that will
// drive the picker (inline for same-service value help, resolved for
// reference-backed), and the service path to fetch rows from (the
// current service's for inline, the F4 service's for references).
let _vlActiveProperty = null;
let _vlActiveValueList = null;
let _vlActiveServicePath = null;
// Cache resolved references across opens so reopening the same F4 is
// instant. Key: absolute reference URL (NOT property name — the same
// F4 can back many properties).
const _vlResolveCache = new Map();

async function openValueListPicker(propertyName) {
  const tab = getActiveTab();
  const info = tab && tab._lastDescribeInfo;
  if (!info) return;
  const prop = info.properties.find(p => p.name === propertyName);
  if (!prop) return;
  const hasInline = !!prop.value_list;
  const hasRefs = Array.isArray(prop.value_list_references) && prop.value_list_references.length > 0;
  const fixedOnly = !hasInline && !hasRefs && prop.value_list_fixed === true;
  if (fixedOnly) {
    // Nothing to drive a picker with — surface the hint in-line rather
    // than open an empty modal. Borrow the status bar briefly.
    setStatus('This property has fixed values but no ValueList mapping in this service.');
    return;
  }
  if (!hasInline && !hasRefs) return;
  _vlActiveProperty = prop;
  const modal = document.getElementById('valueListModal');
  const title = document.getElementById('vlTitle');
  const subtitle = document.getElementById('vlSubtitle');
  const mapping = document.getElementById('vlMapping');
  const filter = document.getElementById('vlFilter');
  const results = document.getElementById('vlResults');
  const status = document.getElementById('vlStatus');
  modal.classList.remove('hidden');
  if (hasInline) {
    _vlActiveValueList = prop.value_list;
    _vlActiveServicePath = currentServicePath;
    title.textContent = prop.value_list.label || `Value Help · ${prop.name}`;
    subtitle.textContent = prop.value_list.collection_path;
    mapping.textContent = `Mapping: ${valueListSummary(prop.value_list)}`;
    filter.value = buildInitialVlFilter(prop, info, prop.value_list);
    results.innerHTML = '<div class="p-4 text-ox-dim text-[11px]">Click Fetch to load values.</div>';
    status.textContent = 'Ready.';
    setTimeout(() => filter.focus(), 0);
    return;
  }
  // Reference-backed: resolve on open. Try references in order until
  // one succeeds; most services have a single reference.
  title.textContent = `Value Help · ${prop.name}`;
  subtitle.textContent = 'resolving reference…';
  mapping.textContent = '';
  filter.value = '';
  results.innerHTML = '<div class="p-4 text-ox-dim text-[11px]">Resolving value-help service…</div>';
  status.textContent = 'Resolving…';
  let resolved = null;
  let lastErr = null;
  for (const refUrl of prop.value_list_references) {
    const cached = _vlResolveCache.get(refUrl);
    if (cached) { resolved = cached; break; }
    try {
      // timedInvoke unwraps the { data, trace } envelope and applies the
      // trace to the current tab so the Inspector shows the F4 fetch.
      resolved = await timedInvoke('resolve_value_list_reference', {
        profileName: currentProfile,
        servicePath: currentServicePath,
        referenceUrl: refUrl,
        localProperty: prop.name,
      });
      _vlResolveCache.set(refUrl, resolved);
      break;
    } catch (e) {
      lastErr = e;
    }
  }
  if (!resolved) {
    status.textContent = 'Resolve error';
    results.innerHTML = `<div class="p-4 text-ox-red text-[11px]">Could not resolve reference.\n${escapeHtml(String(lastErr || 'unknown error'))}</div>`;
    return;
  }
  _vlActiveValueList = resolved.value_list;
  _vlActiveServicePath = resolved.resolved_service_path;
  title.textContent = resolved.value_list.label || `Value Help · ${prop.name}`;
  subtitle.textContent = resolved.value_list.collection_path;
  mapping.textContent = `Mapping: ${valueListSummary(resolved.value_list)}`;
  filter.value = buildInitialVlFilter(prop, info, resolved.value_list);
  results.innerHTML = '<div class="p-4 text-ox-dim text-[11px]">Click Fetch to load values.</div>';
  status.textContent = `Resolved → ${resolved.resolved_service_path}`;
  setTimeout(() => filter.focus(), 0);
}

function closeValueListPicker() {
  document.getElementById('valueListModal').classList.add('hidden');
  _vlActiveProperty = null;
  _vlActiveValueList = null;
  _vlActiveServicePath = null;
}

// Pre-seed the VL filter with In/InOut parameter constraints by
// echoing whatever values the main $filter already has on those
// local columns. A lightweight text match — we look for
// `LocalProperty eq <literal>` or `LocalProperty eq '<literal>'`
// patterns. Anything fancier stays empty and the user can type.
function buildInitialVlFilter(prop, info, vl) {
  const mainFilter = (document.getElementById('qFilter').value || '').trim();
  if (!mainFilter) return '';
  const clauses = [];
  for (const param of vl.parameters) {
    if (param.kind !== 'in' && param.kind !== 'inout') continue;
    if (!param.local_property) continue;
    const re = new RegExp(`\\b${param.local_property}\\s+eq\\s+('[^']*'|[-\\w.]+)`);
    const m = mainFilter.match(re);
    if (m) {
      clauses.push(`${param.value_list_property} eq ${m[1]}`);
    }
    if (param.kind === 'inout' && param.local_property === prop.name) {
      // We're opening this picker specifically because the user wants
      // to set `prop`. Don't pre-filter by its own current value.
      clauses.pop();
    }
  }
  // Echo Constant parameters unconditionally — they're fixed filters.
  for (const param of vl.parameters) {
    if (param.kind === 'constant' && param.constant !== null && param.constant !== undefined) {
      const needsQuotes = isNaN(Number(param.constant)) && param.constant !== 'true' && param.constant !== 'false';
      const lit = needsQuotes ? `'${param.constant.replace(/'/g, "''")}'` : param.constant;
      clauses.push(`${param.value_list_property} eq ${lit}`);
    }
  }
  void info;
  return clauses.join(' and ');
}

async function fetchValueListRows() {
  const prop = _vlActiveProperty;
  const vl = _vlActiveValueList;
  const servicePath = _vlActiveServicePath;
  if (!prop || !vl || !servicePath) return;
  const filter = document.getElementById('vlFilter').value.trim();
  const top = parseInt(document.getElementById('vlTop').value, 10) || 100;
  const status = document.getElementById('vlStatus');
  const results = document.getElementById('vlResults');
  status.textContent = 'Fetching…';
  results.innerHTML = '<div class="p-4 text-ox-dim text-[11px]">Loading…</div>';
  try {
    const params = {
      entity_set: vl.collection_path,
      filter: filter || null,
      top,
    };
    const data = await timedInvoke('run_query', {
      profileName: currentProfile,
      servicePath,
      params,
    });
    renderValueListRows(data, prop, vl);
  } catch (e) {
    status.textContent = 'Fetch error';
    results.innerHTML = `<div class="p-4 text-ox-red text-[11px]">${escapeHtml(String(e))}</div>`;
  }
}

function renderValueListRows(data, prop, vl) {
  const rows = extractRows(data) || [];
  const status = document.getElementById('vlStatus');
  const results = document.getElementById('vlResults');
  status.textContent = `${rows.length} row(s).`;
  if (rows.length === 0) {
    results.innerHTML = '<div class="p-4 text-ox-dim text-[11px]">No rows.</div>';
    return;
  }
  // Column order: prioritize ValueListProperty names from the mapping
  // so the picker feels task-shaped; then fill in any remaining keys.
  void prop;
  const priority = vl.parameters.map(p => p.value_list_property);
  const firstRowKeys = Object.keys(rows[0]).filter(k => !k.startsWith('__'));
  const cols = [];
  for (const k of priority) if (firstRowKeys.includes(k) && !cols.includes(k)) cols.push(k);
  for (const k of firstRowKeys) if (!cols.includes(k)) cols.push(k);
  let html = '<table class="w-full"><thead class="sticky top-0 bg-ox-surface z-10"><tr class="text-ox-dim text-[10px]">';
  for (const c of cols) {
    html += `<th class="text-left px-3 py-1 border-b border-ox-border">${escapeHtml(c)}</th>`;
  }
  html += '</tr></thead><tbody>';
  for (let i = 0; i < rows.length; i++) {
    html += `<tr class="hover:bg-ox-electric/10 cursor-pointer border-b border-ox-border/30" data-action="vl-pick" data-row="${i}">`;
    for (const c of cols) {
      const v = rows[i][c];
      const shown = v === null || v === undefined ? '' : String(v);
      html += `<td class="px-3 py-1 text-ox-text">${escapeHtml(shown)}</td>`;
    }
    html += '</tr>';
  }
  html += '</tbody></table>';
  results.innerHTML = html;
  // Stash rows on the container so the pick handler can read without
  // re-fetching. Use a private key to avoid clashing with user data.
  results._vlRows = rows;
}

function pickValueListRow(rowIndex) {
  const prop = _vlActiveProperty;
  const vl = _vlActiveValueList;
  const results = document.getElementById('vlResults');
  if (!prop || !vl || !results._vlRows) return;
  const row = results._vlRows[rowIndex];
  if (!row) return;
  const tab = getActiveTab();
  const info = tab && tab._lastDescribeInfo;
  if (!info) return;
  const clauses = [];
  for (const param of vl.parameters) {
    // InOut and Out bind picked VL values back onto local properties.
    // In parameters are one-way (local → VL) and don't get written back.
    if (param.kind !== 'inout' && param.kind !== 'out') continue;
    if (!param.local_property) continue;
    const value = row[param.value_list_property];
    if (value === null || value === undefined) continue;
    const localProp = info.properties.find(p => p.name === param.local_property);
    const lit = formatODataLiteral(value, localProp ? localProp.edm_type : 'Edm.String');
    clauses.push(`${param.local_property} eq ${lit}`);
  }
  if (clauses.length === 0) {
    closeValueListPicker();
    return;
  }
  // Merge into main $filter with `and`. If any of our clauses is
  // already present verbatim, drop it — this matters on the common
  // "user opens picker, picks same row twice" case.
  const filterInput = document.getElementById('qFilter');
  const existing = (filterInput.value || '').trim();
  const merged = existing
    ? [existing, ...clauses.filter(c => !existing.includes(c))].join(' and ')
    : clauses.join(' and ');
  filterInput.value = merged;
  closeValueListPicker();
  filterInput.focus();
}

// OData v4 literal formatting by edm type — enough for the picker's
// `local eq <lit>` clauses. Falls back to single-quoted strings for
// unknown types since that's what SAP services overwhelmingly expect.
function formatODataLiteral(value, edmType) {
  const t = (edmType || '').replace('Edm.', '');
  const s = String(value);
  switch (t) {
    case 'Boolean':
      return s === 'true' || s === 'false' ? s : `'${s.replace(/'/g, "''")}'`;
    case 'Byte':
    case 'SByte':
    case 'Int16':
    case 'Int32':
    case 'Int64':
    case 'Decimal':
    case 'Double':
    case 'Single':
      return s;
    case 'Guid':
      return `guid'${s}'`;
    case 'DateTime':
      return `datetime'${s}'`;
    case 'DateTimeOffset':
      return s;
    default:
      return `'${s.replace(/'/g, "''")}'`;
  }
}

// Compact summary of a ValueList's parameter bindings for the marker
// tooltip, e.g. "Warehouse↔Warehouse, Plant→Plant, Language=EN, (Desc)".
// Kept short on purpose — the picker modal shows the full mapping.
function valueListSummary(vl) {
  if (!vl || !Array.isArray(vl.parameters)) return '';
  const bits = vl.parameters.map(p => {
    switch (p.kind) {
      case 'inout':
        return `${p.local_property}↔${p.value_list_property}`;
      case 'in':
        return `${p.local_property}→${p.value_list_property}`;
      case 'out':
        return `${p.value_list_property}→${p.local_property}`;
      case 'constant':
        return `${p.value_list_property}=${p.constant ?? ''}`;
      case 'displayonly':
        return `(${p.value_list_property})`;
      default:
        return p.value_list_property || '?';
    }
  });
  return bits.join(', ');
}

// SAP View marker for properties that have a value help. Covers three
// shapes: inline Common.ValueList, Common.ValueListReferences (URLs to
// separate F4 services), and Common.ValueListWithFixedValues (a marker
// that says "few fixed values"; no mapping to drive, so the picker
// can't offer a real picker — we still show the badge as a hint).
// The button is a self-contained action inside the row — delegated
// handler's `closest('[data-action]')` picks this over the row's
// `select` action so clicking the marker does NOT also add the column
// to `$select`.
function valueListHint(p) {
  if (!sapViewEnabled) return '';
  const hasInline = !!p.value_list;
  const refs = Array.isArray(p.value_list_references) ? p.value_list_references : [];
  const hasRefs = refs.length > 0;
  const fixed = p.value_list_fixed === true;
  if (!hasInline && !hasRefs && !fixed) return '';
  // Inline > references > fixed-only for richness — pick class + tooltip
  // accordingly. Fixed-only gets the mutest variant since it's just a
  // hint with no picker target.
  let cls;
  let tip;
  let kind;
  if (hasInline) {
    const vl = p.value_list;
    const label = vl.label ? `${vl.label}\n` : '';
    tip = `${label}Value help → ${vl.collection_path}${vl.search_supported === true ? ' ($search)' : ''}\n${valueListSummary(vl)}`;
    cls = 'text-ox-electric border border-ox-electric/50 hover:bg-ox-electric/10 hover:border-ox-electric';
    kind = 'inline';
  } else if (hasRefs) {
    tip = `Referenced value help (${refs.length} ref${refs.length > 1 ? 's' : ''}) — resolved on open:\n${refs.join('\n')}`;
    // Dashed border signals "external reference, resolution required".
    cls = 'text-ox-electric border border-dashed border-ox-electric/60 hover:bg-ox-electric/10 hover:border-ox-electric';
    kind = 'refs';
  } else {
    // Fixed-values only — no mapping, just a Fiori "dropdown-worthy" hint.
    tip = 'Common.ValueListWithFixedValues — property has a fixed value set but no ValueList mapping in this service.';
    cls = 'text-ox-dim border border-ox-dim/50 cursor-help';
    kind = 'fixed';
  }
  return ` <button type="button" class="text-[9px] font-semibold tracking-wide px-1 py-px rounded-sm ${cls} transition-colors align-middle" data-action="value-list" data-prop="${escapeHtml(p.name)}" data-kind="${kind}" title="${escapeHtml(tip)}">&#x21D2; F4</button>`;
}

// Show the "Fiori cols" button when SAP View is on and UI.LineItem is
// present. Filter to DataFields whose value_path is an actual property
// on this entity so V4 $select stays valid — nav-path DataFields are
// skipped (they belong in $expand).
function renderFioriColsButton(info) {
  const btn = document.getElementById('btnFioriCols');
  if (!btn) return;
  const active = sapViewEnabled && info;
  const fields = active && Array.isArray(info.line_item) ? info.line_item : [];
  const propNames = new Set(
    Array.isArray(info && info.properties) ? info.properties.map(p => p.name) : []
  );
  // LineItem value_paths (the visible columns Fiori would show).
  const linePaths = fields
    .map(f => f && f.value_path)
    .filter(p => typeof p === 'string' && propNames.has(p));
  // PresentationVariant.RequestAtLeast (the silent "always include" set
  // — time zones, description keys, etc.). Appended AFTER the LineItem
  // columns so they don't disturb the positional order a Fiori list
  // report would use. Dedupe against line_item to avoid double-listing.
  const lineSet = new Set(linePaths);
  const requestAtLeast = active && Array.isArray(info.request_at_least)
    ? info.request_at_least.filter(p => propNames.has(p) && !lineSet.has(p))
    : [];
  const paths = [...linePaths, ...requestAtLeast];
  if (paths.length === 0) {
    btn.classList.add('hidden');
    btn.removeAttribute('data-paths');
    return;
  }
  btn.classList.remove('hidden');
  // Label notes the RequestAtLeast augment when it kicked in so the
  // user understands why the select is wider than the visible cols.
  const suffix = requestAtLeast.length ? ` +${requestAtLeast.length}` : '';
  btn.textContent = `Fiori cols (${linePaths.length}${suffix})`;
  const tipBase = 'Populate $select with UI.LineItem default columns (Fiori list report).';
  btn.title = requestAtLeast.length
    ? `${tipBase}\nIncludes ${requestAtLeast.length} UI.PresentationVariant.RequestAtLeast field(s).`
    : tipBase;
  btn.dataset.paths = paths.join(',');
}

// Replace $select with the Fiori LineItem defaults. We overwrite rather
// than append — the point of this action is "show me what Fiori shows",
// and merging would defeat that.
function applyFioriCols() {
  const btn = document.getElementById('btnFioriCols');
  const input = document.getElementById('qSelect');
  if (!btn || !input) return;
  const paths = (btn.dataset.paths || '').split(',').filter(Boolean);
  if (paths.length === 0) return;
  input.value = paths.join(',');
  input.focus();
}

// UI.Criticality → small badge. Fixed levels map to the OData spec
// (0 Neutral, 1 Negative, 2 Critical, 3 Positive, 5 Information). Path
// criticality renders as "⇢ TargetProp" so the user can see where the
// value comes from at runtime.
function criticalityHint(p) {
  const c = p.criticality;
  if (!c) return '';
  if (c.kind === 'fixed') {
    const level = c.value;
    const color = level === 3 ? 'text-ox-green' :
                  level === 2 ? 'text-ox-amber' :
                  level === 1 ? 'text-ox-red' :
                  level === 5 ? 'text-ox-blue' : 'text-ox-dim';
    const label = level === 3 ? 'positive' :
                  level === 2 ? 'critical' :
                  level === 1 ? 'negative' :
                  level === 5 ? 'info' : 'neutral';
    return ` <span class="${color} text-[10px]" title="UI.Criticality = ${label}">&#9679;</span>`;
  }
  if (c.kind === 'path') {
    return ` <span class="text-ox-blue text-[10px]" title="UI.Criticality Path = ${escapeHtml(c.value)}">&#8680; ${escapeHtml(c.value)}</span>`;
  }
  return '';
}

function renderDescribe(info) {
  const panel = document.getElementById('describePanel');
  panel.classList.remove('hidden');

  // Cache so the SAP-view toggle can re-render without another fetch.
  const tab = getActiveTab();
  if (tab) tab._lastDescribeInfo = info;

  // Selection-fields chip bar above the query inputs (SAP View only).
  renderSelectionFieldsBar(info);
  // UI.LineItem → "Fiori cols" quick-action button next to $select.
  renderFioriColsButton(info);

  // Title: technical name always; in SAP view, append the HeaderInfo
  // singular/plural so the entity reads as "WarehouseOrderType · Warehouse Order / Warehouse Orders".
  const titleEl = document.getElementById('entityTitle');
  titleEl.textContent = info.name;
  if (sapViewEnabled && info.header_info) {
    const hi = info.header_info;
    const parts = [];
    if (hi.type_name) parts.push(hi.type_name);
    if (hi.type_name_plural && hi.type_name_plural !== hi.type_name) parts.push(hi.type_name_plural);
    if (parts.length) {
      titleEl.innerHTML =
        `${escapeHtml(info.name)}` +
        `<span class="text-ox-dim ml-2">·</span>` +
        `<span class="text-ox-blue ml-2">${escapeHtml(parts.join(' / '))}</span>`;
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
    const textHint = sapViewEnabled && p.text_path
      ? ` <span class="text-ox-blue text-[10px]" title="Common.Text → ${escapeHtml(p.text_path)}">&#x21A6; ${escapeHtml(p.text_path)}</span>`
      : '';
    const currencyHint = sapViewEnabled && p.iso_currency_path
      ? ` <span class="text-ox-green text-[10px]" title="Measures.ISOCurrency → ${escapeHtml(p.iso_currency_path)}">&curren; ${escapeHtml(p.iso_currency_path)}</span>`
      : '';
    const unitHint = sapViewEnabled && p.unit_path && !p.iso_currency_path
      ? ` <span class="text-ox-green text-[10px]" title="Measures.Unit / sap:unit → ${escapeHtml(p.unit_path)}">&#8593; ${escapeHtml(p.unit_path)}</span>`
      : '';
    const titleHint = sapViewEnabled && info.header_info && info.header_info.title_path === p.name
      ? ' <span class="text-ox-amber text-[10px]" title="Used as UI.HeaderInfo.Title">title</span>'
      : '';
    const flagHints = sapViewEnabled ? propertyFlagHints(p) : '';
    const critHint = sapViewEnabled ? criticalityHint(p) : '';
    const vlHint = valueListHint(p);
    html += `<tr class="hover:bg-ox-amberGlow cursor-pointer transition-colors" data-action="select" data-field="${escapeHtml(p.name)}">`;
    html += `<td class="py-0.5 pr-3 text-ox-text">${escapeHtml(p.name)}${textHint}${currencyHint}${unitHint}${titleHint}${critHint}${flagHints}${vlHint}</td>`;
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
  document.getElementById('describeContent').innerHTML = html;
}

function hideDescribe() {
  document.getElementById('describePanel').classList.add('hidden');
}

// ══════════════════════════════════════════════════════════════
// ── CLICK-TO-ADD HELPERS ──
// ══════════════════════════════════════════════════════════════

function addToSelect(fieldName) {
  const el = document.getElementById('qSelect');
  const current = el.value.split(',').map(s => s.trim()).filter(Boolean);
  if (!current.includes(fieldName)) {
    current.push(fieldName);
    el.value = current.join(',');
  }
}

function addToExpand(navName) {
  const el = document.getElementById('qExpand');
  const current = el.value.split(',').map(s => s.trim()).filter(Boolean);
  if (!current.includes(navName)) {
    current.push(navName);
    el.value = current.join(',');
  }
}

// ══════════════════════════════════════════════════════════════
// ── ODATA URL BUILDER (for Copy URL feature) ──
// ══════════════════════════════════════════════════════════════

function buildODataUrl(params) {
  if (!currentServicePath || !params) return '';
  const parts = [];
  if (params.select)  parts.push(`$select=${encodeURIComponent(params.select)}`);
  if (params.filter)  parts.push(`$filter=${encodeURIComponent(params.filter)}`);
  if (params.expand)  parts.push(`$expand=${encodeURIComponent(params.expand)}`);
  if (params.orderby) parts.push(`$orderby=${encodeURIComponent(params.orderby)}`);
  if (params.top)     parts.push(`$top=${params.top}`);
  if (params.skip)    parts.push(`$skip=${params.skip}`);
  const qs = parts.length ? '?' + parts.join('&') : '';
  return `${currentServicePath}/${params.entity_set}${qs}`;
}

// ══════════════════════════════════════════════════════════════
// ── QUERY EXECUTION ──
// ══════════════════════════════════════════════════════════════

async function executeQuery(asJson = false) {
  if (!currentProfile || !currentServicePath || !currentEntitySet) {
    setStatus('Select a profile, service, and entity set first');
    return;
  }

  const params = {
    entity_set: currentEntitySet,
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
  if (sapViewEnabled) {
    const tab = getActiveTab();
    const info = tab && tab._lastDescribeInfo;
    if (info && info.name && currentEntitySet === document.getElementById('queryEntitySet').textContent) {
      showSapViewWarnings(validateQueryRestrictions(params, info));
    } else {
      showSapViewWarnings([]);
    }
  } else {
    showSapViewWarnings([]);
  }

  setStatus(`Querying ${currentEntitySet}...`);
  const queryStart = performance.now();

  try {
    const data = await timedInvoke('run_query', {
      profileName: currentProfile,
      servicePath: currentServicePath,
      params,
    });

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
    setStatus('Query error: ' + e);
    hideStatsBar();
    const message = isBrowserAuthProfile(currentProfile) ? browserAuthMessage(e) : String(e);
    document.getElementById('resultsArea').innerHTML =
      `<div class="p-4 text-ox-red text-sm">${escapeHtml(message)}</div>`;
  }
}

// ══════════════════════════════════════════════════════════════
// ── STATS BAR (Feature 4) ──
// ══════════════════════════════════════════════════════════════

function showStatsBar(rowCount, sizeBytes, elapsedMs) {
  document.getElementById('statRows').textContent = `${rowCount} row${rowCount !== 1 ? 's' : ''}`;
  document.getElementById('statSize').textContent = formatBytes(sizeBytes);

  let timingClass = 'timing-fast';
  if (elapsedMs >= 2000) timingClass = 'timing-slow';
  else if (elapsedMs >= 500) timingClass = 'timing-ok';

  document.getElementById('statTiming').innerHTML =
    `<span class="${timingClass}">${elapsedMs}ms</span>`;
  document.getElementById('statsBar').classList.remove('hidden');
}

function hideStatsBar() {
  document.getElementById('statsBar').classList.add('hidden');
}

function formatBytes(bytes) {
  if (bytes < 1024) return bytes + ' B';
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
  return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
}

// ══════════════════════════════════════════════════════════════
// ── QUERY HISTORY (Feature 3) ──
// ══════════════════════════════════════════════════════════════

function addToHistory(tab, params, rowCount, elapsed) {
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

function renderHistoryPanel(tab) {
  const panel = document.getElementById('historyPanel');
  if (!tab || tab.queryHistory.length === 0) {
    panel.innerHTML = '<div class="px-4 py-3 text-[11px] text-ox-dim font-mono">No history yet</div>';
    return;
  }
  let html = '<div class="flex items-center justify-between px-3 py-1 border-b border-ox-border">';
  html += '<span class="text-[9px] uppercase tracking-widest text-ox-dim font-medium">Query History</span>';
  html += '<button data-action="clear-history" class="text-[10px] text-ox-dim hover:text-ox-red px-1 transition-colors">clear</button>';
  html += '</div>';
  for (let i = 0; i < tab.queryHistory.length; i++) {
    const h = tab.queryHistory[i];
    const time = h.ts.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
    html += `<div class="history-item" data-action="replay-history" data-idx="${i}">
      <span class="text-ox-amber shrink-0">${escapeHtml(h.entitySet)}</span>
      <span class="text-ox-dim flex-1 truncate">${escapeHtml(h.summary)}</span>
      <span class="text-ox-dim shrink-0">${h.rowCount}r</span>
      <span class="text-ox-dim shrink-0">${h.elapsed}ms</span>
      <span class="text-ox-dim shrink-0">${time}</span>
    </div>`;
  }
  panel.innerHTML = html;
}

function replayHistory(idx) {
  const tab = getActiveTab();
  if (!tab) return;
  const h = tab.queryHistory[idx];
  if (!h) return;
  // Restore entity set and params into query bar
  if (h.entitySet) {
    currentEntitySet = h.entitySet;
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

// ══════════════════════════════════════════════════════════════
// ── HTTP INSPECTOR ──
// ══════════════════════════════════════════════════════════════

function clearTraceState(tab = getActiveTab()) {
  if (!tab) return;
  tab.httpTraceEntries = [];
  tab.selectedTraceId = null;
  tab._traceVisible = false;
  if (tab.id === activeTabId) {
    document.getElementById('traceInspectorPanel').classList.add('hidden');
    renderTraceSummary(tab);
  }
}

function ensureTraceSelection(tab) {
  if (!tab) return null;
  if (
    tab.selectedTraceId &&
    tab.httpTraceEntries.some(entry => entry.id === tab.selectedTraceId)
  ) {
    return tab.selectedTraceId;
  }
  tab.selectedTraceId = tab.httpTraceEntries.length
    ? tab.httpTraceEntries[tab.httpTraceEntries.length - 1].id
    : null;
  return tab.selectedTraceId;
}

function getSelectedTraceEntry(tab = getActiveTab()) {
  if (!tab) return null;
  const selectedId = ensureTraceSelection(tab);
  if (!selectedId) return null;
  return tab.httpTraceEntries.find(entry => entry.id === selectedId) || null;
}

function renderTraceSummary(tab = getActiveTab()) {
  const count = tab?.httpTraceEntries?.length || 0;
  document.getElementById('traceSummary').textContent = count
    ? `${count} request${count === 1 ? '' : 's'}`
    : 'No trace';
  document.getElementById('traceCount').textContent = count
    ? `${count} request${count === 1 ? '' : 's'}`
    : 'No requests';
}

function traceStatusClass(entry) {
  if (entry.error) return 'err';
  if (!entry.status) return '';
  if (entry.status >= 500) return 'err';
  if (entry.status >= 400) return 'warn';
  if (entry.status >= 300) return 'warn';
  return 'ok';
}

function traceStatusLabel(entry) {
  if (entry.status) return String(entry.status);
  if (entry.error) return 'ERR';
  return 'OPEN';
}

function compactTraceUrl(url) {
  try {
    const parsed = new URL(url);
    return `${parsed.host}${parsed.pathname}${parsed.search}`;
  } catch {
    return url;
  }
}

function traceOutcomeLabel(entry) {
  if (entry.error) return entry.error;
  if (entry.redirect_location) return `redirect → ${entry.redirect_location}`;
  return entry.response_body_preview ? 'response captured' : 'headers captured';
}

function renderTraceHeaders(headers) {
  if (!headers || headers.length === 0) {
    return '<div class="trace-code text-ox-dim">No headers captured.</div>';
  }

  let html = '<div class="trace-header-grid">';
  for (const header of headers) {
    html += `<div class="trace-header-name">${escapeHtml(header.name)}</div>`;
    html += `<div class="trace-header-value">${escapeHtml(header.value)}</div>`;
  }
  html += '</div>';
  return html;
}

function renderTraceBody(body, emptyLabel) {
  if (!body) {
    return `<div class="trace-code text-ox-dim">${escapeHtml(emptyLabel)}</div>`;
  }
  return `<pre class="trace-code">${escapeHtml(body)}</pre>`;
}

function renderTraceList(tab) {
  const list = document.getElementById('traceList');
  if (!tab || tab.httpTraceEntries.length === 0) {
    list.innerHTML = '<div class="px-4 py-3 text-[11px] text-ox-dim font-mono">No traced requests yet.</div>';
    return;
  }

  const selectedId = ensureTraceSelection(tab);
  let html = '';
  for (const entry of [...tab.httpTraceEntries].reverse()) {
    const active = entry.id === selectedId ? ' active' : '';
    const statusClass = traceStatusClass(entry);
    const statusCls = statusClass ? ` ${statusClass}` : '';
    html += `<div class="trace-row${active}" data-action="select-trace" data-trace-id="${entry.id}">`;
    html += '<div class="trace-meta">';
    html += `<span class="trace-pill">${escapeHtml(entry.method)}</span>`;
    html += `<span class="trace-pill${statusCls}">${escapeHtml(traceStatusLabel(entry))}</span>`;
    html += `<span>${entry.duration_ms}ms</span>`;
    html += '</div>';
    html += `<div class="trace-url">${escapeHtml(compactTraceUrl(entry.url))}</div>`;
    html += `<div class="trace-meta">${escapeHtml(traceOutcomeLabel(entry))}</div>`;
    html += '</div>';
  }
  list.innerHTML = html;
}

function renderTraceDetail(tab) {
  const detail = document.getElementById('traceDetail');
  const entry = getSelectedTraceEntry(tab);
  if (!entry) {
    detail.innerHTML = '<div class="px-4 py-4 text-[11px] text-ox-dim font-mono">Select a traced request to inspect it.</div>';
    return;
  }

  const activeSubTab = tab?._traceSubTab === 'request' ? 'request' : 'response';
  const statusClass = traceStatusClass(entry);
  const statusCls = statusClass ? ` ${statusClass}` : '';

  let html = '<div class="trace-section">';
  html += '<div class="flex items-center gap-2 mb-2">';
  html += `<span class="trace-pill">${escapeHtml(entry.method)}</span>`;
  html += `<span class="trace-pill${statusCls}">${escapeHtml(traceStatusLabel(entry))}</span>`;
  html += `<span class="trace-meta">${entry.duration_ms}ms</span>`;
  html += '</div>';
  html += `<div class="trace-url">${escapeHtml(entry.url)}</div>`;
  html += '</div>';

  html += '<div class="trace-subtabs">';
  html += `<div class="trace-subtab${activeSubTab === 'request' ? ' active' : ''}" data-action="select-trace-subtab" data-subtab="request">Request</div>`;
  html += `<div class="trace-subtab${activeSubTab === 'response' ? ' active' : ''}" data-action="select-trace-subtab" data-subtab="response">Response</div>`;
  html += '<div class="trace-subtab-actions">';
  if (activeSubTab === 'request') {
    html += '<button data-action="copy-trace-curl">copy as curl</button>';
    const disabled = entry.request_body_preview ? '' : ' disabled';
    html += `<button data-action="copy-trace-request-body"${disabled}>copy body</button>`;
  } else {
    const disabled = entry.response_body_preview ? '' : ' disabled';
    html += `<button data-action="copy-trace-response-body"${disabled}>copy body</button>`;
  }
  html += '</div>';
  html += '</div>';

  if (activeSubTab === 'request') {
    html += '<div class="trace-section">';
    html += '<div class="trace-section-title">Headers</div>';
    html += renderTraceHeaders(entry.request_headers);
    html += '</div>';

    html += '<div class="trace-section">';
    html += '<div class="trace-section-title">Body</div>';
    html += renderTraceBody(entry.request_body_preview, 'No request body captured.');
    html += '</div>';
  } else {
    html += '<div class="trace-section">';
    html += '<div class="trace-section-title">Headers</div>';
    html += renderTraceHeaders(entry.response_headers);
    html += '</div>';

    html += '<div class="trace-section">';
    html += '<div class="trace-section-title">Body Preview</div>';
    html += renderTraceBody(entry.response_body_preview, 'No response body preview captured.');
    html += '</div>';

    if (entry.redirect_location) {
      html += '<div class="trace-section">';
      html += '<div class="trace-section-title">Redirect</div>';
      html += `<div class="trace-code">${escapeHtml(entry.redirect_location)}</div>`;
      html += '</div>';
    }

    if (entry.error) {
      html += '<div class="trace-section">';
      html += '<div class="trace-section-title">Error</div>';
      html += `<pre class="trace-code">${escapeHtml(entry.error)}</pre>`;
      html += '</div>';
    }
  }

  detail.innerHTML = html;
}

function renderTraceInspector(tab = getActiveTab()) {
  renderTraceSummary(tab);
  renderTraceList(tab);
  renderTraceDetail(tab);
}

function showTraceInspector() {
  const tab = getActiveTab();
  if (!tab) return;
  tab._traceVisible = true;
  renderTraceInspector(tab);
  document.getElementById('traceInspectorPanel').classList.remove('hidden');
  updateTraceToggleState(true);
}

function hideTraceInspector() {
  const tab = getActiveTab();
  if (tab) tab._traceVisible = false;
  document.getElementById('traceInspectorPanel').classList.add('hidden');
  updateTraceToggleState(false);
}

function updateTraceToggleState(open) {
  const btn = document.getElementById('btnTraceToggle');
  const chevron = document.getElementById('traceToggleChevron');
  if (!btn || !chevron) return;
  chevron.innerHTML = open ? '&#x25BE;' : '&#x25B4;';
  if (open) {
    btn.classList.add('text-ox-amber', 'border-ox-amber');
    btn.classList.remove('text-ox-dim', 'border-ox-border');
  } else {
    btn.classList.add('text-ox-dim', 'border-ox-border');
    btn.classList.remove('text-ox-amber', 'border-ox-amber');
  }
}

function updateSapViewToggleState() {
  const btn = document.getElementById('btnSapView');
  const chev = document.getElementById('sapViewChevron');
  if (!btn || !chev) return;
  chev.innerHTML = sapViewEnabled ? '&#x25BE;' : '&#x25B4;';
  // Off state = dim amber (primed but inactive). On state = full amber
  // with a subtle glow so it reads clearly as "engaged".
  if (sapViewEnabled) {
    btn.classList.add('text-ox-amber', 'border-ox-amber', 'bg-ox-amberGlow');
    btn.classList.remove('text-ox-amberDim', 'border-ox-amberDim/60');
  } else {
    btn.classList.add('text-ox-amberDim', 'border-ox-amberDim/60');
    btn.classList.remove('text-ox-amber', 'border-ox-amber', 'bg-ox-amberGlow');
  }
}

function toggleSapView() {
  sapViewEnabled = !sapViewEnabled;
  try { localStorage.setItem('ox_sap_view_enabled', sapViewEnabled ? '1' : '0'); } catch { /* ignore */ }
  updateSapViewToggleState();
  // Re-render describe panel in place if the active tab has one up.
  // renderDescribe also refreshes the selection-fields chip bar.
  const tab = getActiveTab();
  if (tab && tab._lastDescribeInfo) {
    renderDescribe(tab._lastDescribeInfo);
  } else {
    // No cached describe — still hide any stale chip bar and quick-action.
    renderSelectionFieldsBar(null);
    renderFioriColsButton(null);
  }
  // Clear any lingering warnings from the previous mode.
  if (!sapViewEnabled) showSapViewWarnings([]);
}

function toggleTraceInspector() {
  const panel = document.getElementById('traceInspectorPanel');
  if (panel.classList.contains('hidden')) {
    showTraceInspector();
  } else {
    hideTraceInspector();
  }
}

// POSIX-shell single-quote escape. The resulting curl command runs in bash /
// zsh / git-bash, but cmd.exe and PowerShell use different quoting rules —
// paste into those shells and the quotes will leak through literally.
function shellQuoteForCurl(value) {
  return "'" + String(value).replace(/'/g, `'\"'\"'`) + "'";
}

// ══════════════════════════════════════════════════════════════
// ── ANNOTATION BADGE (footer) ──
// ══════════════════════════════════════════════════════════════
// Small status-bar badge showing the raw annotation count for the
// currently loaded service, with a hover breakdown by vocabulary
// namespace. The thin slice — the typed feature views (criticality,
// Text arrangement, Fiori-readiness, etc.) will layer on top.

function renderAnnotationBadge(summary) {
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
  el.title = lines.length ? lines.join('\n') : `${total} annotations`;
}

function traceToCurl(entry) {
  const parts = [
    `curl -X ${shellQuoteForCurl(entry.method)}`,
    `--url ${shellQuoteForCurl(entry.url)}`,
  ];
  for (const header of entry.request_headers || []) {
    parts.push(`-H ${shellQuoteForCurl(`${header.name}: ${header.value}`)}`);
  }
  if (entry.request_body_preview) {
    parts.push(`--data-raw ${shellQuoteForCurl(entry.request_body_preview)}`);
  }
  return parts.join(' ');
}

async function copySelectedTraceAsCurl() {
  const entry = getSelectedTraceEntry(getActiveTab());
  if (!entry) {
    setStatus('No trace selected');
    return;
  }
  await copyToClipboard(traceToCurl(entry), 'curl command');
}

async function copySelectedTraceRequestBody() {
  const entry = getSelectedTraceEntry(getActiveTab());
  if (!entry || !entry.request_body_preview) {
    setStatus('No request body to copy');
    return;
  }
  await copyToClipboard(entry.request_body_preview, 'request body');
}

async function copySelectedTraceResponseBody() {
  const entry = getSelectedTraceEntry(getActiveTab());
  if (!entry || !entry.response_body_preview) {
    setStatus('No response body to copy');
    return;
  }
  await copyToClipboard(entry.response_body_preview, 'response body');
}

// ══════════════════════════════════════════════════════════════
// ── RESULTS RENDERING ──
// ══════════════════════════════════════════════════════════════

function extractRows(data) {
  if (data.d) {
    if (data.d.results) return data.d.results;
    return [data.d];
  }
  if (data.value) return data.value;
  return null;
}

function renderResults(data, elapsedMs, params) {
  const rows = extractRows(data);
  if (!rows || rows.length === 0) {
    document.getElementById('resultsArea').innerHTML =
      '<div class="p-4 text-ox-dim text-sm">No results</div>';
    setStatus('No results');
    hideStatsBar();
    return;
  }

  expandedDataStore = {};
  lastResultRows = rows;
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
  const sapShape = sapViewEnabled && info && Array.isArray(info.properties);
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

  let html = '<div class="overflow-auto h-full">';
  html += '<table class="w-full text-xs font-mono border-collapse">';
  html += '<thead><tr>';
  for (const col of allCols) {
    const isNested = nestedCols.includes(col);
    const label = isNested ? `${col} ↗` : col;
    html += `<th class="text-left px-3 py-1.5 bg-ox-panel text-ox-dim border-b border-ox-border font-medium sticky top-0 group">`;
    html += `<span class="mr-1">${escapeHtml(label)}</span>`;
    if (!isNested) {
      // Copy column button (Feature 5)
      html += `<button class="copy-btn" data-action="copy-col" data-col="${escapeHtml(col)}" title="Copy column values">`;
      html += `<svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>`;
      html += `</button>`;
    }
    html += `</th>`;
  }
  // Row copy column header
  html += `<th class="text-left px-2 py-1.5 bg-ox-panel border-b border-ox-border sticky top-0 w-6"></th>`;
  html += '</tr></thead><tbody>';

  for (let i = 0; i < rows.length; i++) {
    const row = rows[i];
    const stripe = i % 2 === 0 ? '' : 'bg-ox-surface/50';
    html += `<tr class="hover:bg-ox-amberGlow border-b border-ox-border/30 transition-colors ${stripe}" data-row-idx="${i}">`;

    for (const col of allCols) {
      const val = row[col];
      if (val === null || val === undefined) {
        html += `<td class="px-3 py-1 text-ox-dim">—</td>`;
      } else if (Array.isArray(val)) {
        const storeKey = `r${i}_${col}`;
        expandedDataStore[storeKey] = val;
        const count = val.length;
        html += `<td class="px-3 py-1"><span class="expand-badge text-[10px] px-1.5 py-0.5 rounded-sm font-mono inline-block" data-action="nested" data-key="${storeKey}" data-col="${escapeHtml(col)}">${count} item${count !== 1 ? 's' : ''}</span></td>`;
      } else if (typeof val === 'object') {
        const storeKey = `r${i}_${col}`;
        expandedDataStore[storeKey] = val;
        html += `<td class="px-3 py-1"><span class="expand-badge text-[10px] px-1.5 py-0.5 rounded-sm font-mono inline-block" data-action="nested" data-key="${storeKey}" data-col="${escapeHtml(col)}">object</span></td>`;
      } else {
        const text = String(val);
        // SAP View: fold the Common.Text companion into this cell per
        // UI.TextArrangement. The raw `text` is still what goes into
        // filter-tooltip data attributes so clicking "equals this value"
        // uses the key, not the description.
        let display = text;
        if (sapShape) {
          const prop = propByName.get(col);
          if (prop && prop.text_path) {
            const companion = row[prop.text_path];
            const companionText = companion === null || companion === undefined
              ? ''
              : String(companion);
            const arr = prop.text_arrangement || 'textfirst';
            if (arr === 'textfirst' && companionText) {
              display = `${companionText} (${text})`;
            } else if (arr === 'textlast' && companionText) {
              display = `${text} (${companionText})`;
            } else if (arr === 'textonly') {
              display = companionText || text;
            }
          }
        }
        html += `<td class="px-3 py-1 text-ox-text whitespace-nowrap cursor-pointer" data-action="cell-click" data-cell-col="${escapeHtml(col)}" data-cell-val="${escapeHtml(text)}">${escapeHtml(display)}</td>`;
      }
    }

    // Row copy button (Feature 5)
    const storeKey = `row_${i}`;
    expandedDataStore[storeKey] = row;
    html += `<td class="px-2 py-1">`;
    html += `<button class="copy-btn row-copy-btn" data-action="copy-row" data-key="${storeKey}" title="Copy row as JSON">`;
    html += `<svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>`;
    html += `</button></td>`;
    html += '</tr>';
  }

  html += '</tbody></table></div>';
  document.getElementById('resultsArea').innerHTML = html;

  // lastResultRows already set above for copy operations

  setStatus(`${rows.length} row(s)${nestedCols.length ? ' — click badges to view expanded data' : ''}`);
}

function showNestedData(storeKey, colName) {
  const data = expandedDataStore[storeKey];
  if (!data) return;

  const rows = Array.isArray(data) ? data : [data];
  if (rows.length === 0) { alert('No nested data'); return; }

  const first = rows[0];
  if (typeof first !== 'object' || first === null) {
    const json = JSON.stringify(data, null, 2);
    showNestedPanel(colName, `<pre class="text-xs font-mono text-ox-text p-3 whitespace-pre">${escapeHtml(json)}</pre>`);
    return;
  }

  const cols = Object.keys(first).filter(k => !k.startsWith('@') && k !== '__metadata');

  let html = '<div class="overflow-auto max-h-64"><table class="w-full text-xs font-mono border-collapse">';
  html += '<thead><tr>';
  for (const c of cols) {
    html += `<th class="text-left px-2 py-1 bg-ox-panel text-ox-dim border-b border-ox-border font-medium sticky top-0">${escapeHtml(c)}</th>`;
  }
  html += '</tr></thead><tbody>';
  for (const row of rows) {
    html += '<tr class="border-b border-ox-border/30">';
    for (const c of cols) {
      const v = row[c];
      const t = (v === null || v === undefined) ? '' : (typeof v === 'object' ? JSON.stringify(v) : String(v));
      html += `<td class="px-2 py-0.5 text-ox-text whitespace-nowrap">${escapeHtml(t)}</td>`;
    }
    html += '</tr>';
  }
  html += '</tbody></table></div>';
  showNestedPanel(colName, html);
}

function showNestedPanel(title, contentHtml) {
  const existing = document.getElementById('nestedPanel');
  if (existing) existing.remove();

  const panel = document.createElement('div');
  panel.id = 'nestedPanel';
  panel.className = 'fixed bottom-8 right-4 w-[600px] bg-ox-panel border border-ox-border rounded-lg shadow-2xl z-40';
  panel.style.animation = 'slideUp 0.2s ease';
  panel.innerHTML = `
    <div class="px-4 py-2 border-b border-ox-border flex items-center justify-between">
      <div class="flex items-center gap-2">
        <div class="w-1.5 h-1.5 rounded-full bg-ox-blue"></div>
        <span class="text-xs font-mono font-medium text-ox-text">${escapeHtml(title)}</span>
      </div>
      <button data-action="close-nested" class="text-ox-dim hover:text-ox-text text-xs px-2 py-0.5 rounded-sm hover:bg-ox-hover">close</button>
    </div>
    <div class="p-2">${contentHtml}</div>
  `;
  document.body.appendChild(panel);
}

function renderJson(data) {
  const json = JSON.stringify(data, null, 2);
  document.getElementById('resultsArea').innerHTML =
    `<pre class="text-xs font-mono text-ox-text p-4 overflow-auto h-full whitespace-pre leading-relaxed">${escapeHtml(json)}</pre>`;
  setStatus('JSON');
}

// ══════════════════════════════════════════════════════════════
// ── COPY FUNCTIONS (Feature 5) ──
// ══════════════════════════════════════════════════════════════

async function copyToClipboard(text, label) {
  try {
    await navigator.clipboard.writeText(text);
    setStatus(`Copied ${label || 'to clipboard'}`);
  } catch (e) {
    setStatus('Copy failed: ' + e);
  }
}

function copyColumnValues(colName) {
  const rows = lastResultRows || [];
  const values = rows.map(r => {
    const v = r[colName];
    return (v === null || v === undefined) ? '' : String(v);
  });
  copyToClipboard(values.join('\n'), `column "${colName}"`);
}

function copyRowAsJson(storeKey) {
  const row = expandedDataStore[storeKey];
  if (!row) return;
  const json = JSON.stringify(row, null, 2);
  copyToClipboard(json, 'row as JSON');
}

function copyODataUrl() {
  const params = {
    entity_set: currentEntitySet,
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
// ── FILTER TOOLTIP (Feature 7) ──
// ══════════════════════════════════════════════════════════════

let filterTooltipTimeout = null;

function showFilterTooltip(col, val, x, y) {
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

function hideFilterTooltip() {
  clearTimeout(filterTooltipTimeout);
  document.getElementById('filterTooltip').style.display = 'none';
}

function applyFilterFromTooltip() {
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

// ══════════════════════════════════════════════════════════════
// ── ADD PROFILE MODAL ──
// ══════════════════════════════════════════════════════════════

function showAddProfileModal() {
  document.getElementById('addProfileModal').classList.remove('hidden');
  document.getElementById('mpName').value = '';
  document.getElementById('mpUrl').value = '';
  document.getElementById('mpClient').value = '100';
  document.getElementById('mpLang').value = 'EN';
  document.getElementById('mpAuthMode').value = 'basic';
  document.getElementById('mpUser').value = '';
  document.getElementById('mpPass').value = '';
  updateAuthModeFields();
  document.getElementById('mpError').classList.add('hidden');
  document.getElementById('mpSuccess').classList.add('hidden');
  document.getElementById('mpName').focus();
}

function updateAuthModeFields() {
  const mode = document.getElementById('mpAuthMode').value;
  document.getElementById('mpCredFields').style.display = mode === 'basic' ? '' : 'none';

  const hint = document.getElementById('mpAuthHint');
  if (mode === 'sso') {
    hint.textContent = 'Uses Windows integrated auth via Kerberos / Negotiate.';
  } else if (mode === 'browser') {
    hint.textContent = 'Opens an in-app sign-in window for Azure AD / SAP IAS style browser authentication.';
  } else {
    hint.textContent = 'Stores the password in Windows Credential Manager.';
  }
}

function hideAddProfileModal() {
  document.getElementById('addProfileModal').classList.add('hidden');
}

async function saveProfileModal() {
  const name     = document.getElementById('mpName').value.trim();
  const url      = document.getElementById('mpUrl').value.trim();
  const client   = document.getElementById('mpClient').value.trim();
  const language = document.getElementById('mpLang').value.trim();
  const authMode = document.getElementById('mpAuthMode').value;
  const user     = authMode === 'basic' ? document.getElementById('mpUser').value.trim() : '';
  const pass     = authMode === 'basic' ? document.getElementById('mpPass').value : '';

  const errEl = document.getElementById('mpError');
  const okEl  = document.getElementById('mpSuccess');
  errEl.classList.add('hidden');
  okEl.classList.add('hidden');

  if (!name || !url) {
    errEl.textContent = 'Profile name and URL are required';
    errEl.classList.remove('hidden');
    return;
  }
  if (authMode === 'basic' && (!user || !pass)) {
    errEl.textContent = 'Username and password are required for basic authentication';
    errEl.classList.remove('hidden');
    return;
  }

  try {
    const msg = await invoke('add_profile', {
      name, baseUrl: url, client, language, authMode, username: user, password: pass,
    });
    okEl.textContent = msg;
    okEl.classList.remove('hidden');
    await loadProfiles();
    document.getElementById('profileSelect').value = name;
    document.getElementById('profileSelect').dispatchEvent(new Event('change'));
    setTimeout(hideAddProfileModal, 800);
  } catch (e) {
    errEl.textContent = String(e);
    errEl.classList.remove('hidden');
  }
}

async function testProfileModal() {
  const url    = document.getElementById('mpUrl').value.trim();
  const client = document.getElementById('mpClient').value.trim();
  const language = document.getElementById('mpLang').value.trim() || 'EN';
  const authMode = document.getElementById('mpAuthMode').value;
  const user   = authMode === 'basic' ? document.getElementById('mpUser').value.trim() : '';
  const pass   = authMode === 'basic' ? document.getElementById('mpPass').value : '';
  const name   = document.getElementById('mpName').value.trim();

  const errEl = document.getElementById('mpError');
  const okEl  = document.getElementById('mpSuccess');
  errEl.classList.add('hidden');
  okEl.classList.add('hidden');

  if (!name || !url) {
    errEl.textContent = 'Fill in name and URL first';
    errEl.classList.remove('hidden');
    return;
  }

  try {
    const msg = await timedInvoke('test_connection', {
      baseUrl: url, client, language, authMode, username: user, password: pass,
    });
    okEl.textContent = msg;
    okEl.classList.remove('hidden');
  } catch (e) {
    errEl.textContent = String(e);
    errEl.classList.remove('hidden');
  }
}

// ══════════════════════════════════════════════════════════════
// ── UTILITY ──
// ══════════════════════════════════════════════════════════════

function escapeHtml(str) {
  const div = document.createElement('div');
  div.textContent = str;
  return div.innerHTML;
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
  document.getElementById('btnFioriCols').addEventListener('click', applyFioriCols);
  document.getElementById('btnVlClose').addEventListener('click', closeValueListPicker);
  document.getElementById('btnVlFetch').addEventListener('click', fetchValueListRows);
  document.getElementById('vlFilter').addEventListener('keydown', (e) => {
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
    addTab({ title: 'New Tab', profile: currentProfile });
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
      renderTraceInspector(tab);
    } else if (action === 'select-trace-subtab') {
      const tab = getActiveTab();
      if (!tab) return;
      tab._traceSubTab = el.dataset.subtab === 'request' ? 'request' : 'response';
      renderTraceDetail(tab);
    } else if (action === 'copy-trace-curl') {
      copySelectedTraceAsCurl();
    } else if (action === 'copy-trace-request-body') {
      copySelectedTraceRequestBody();
    } else if (action === 'copy-trace-response-body') {
      copySelectedTraceResponseBody();
    } else if (action === 'value-list') {
      openValueListPicker(el.dataset.prop);
    } else if (action === 'vl-pick') {
      pickValueListRow(parseInt(el.dataset.row, 10));
    } else if (action === 'selection-field') {
      // Seed $filter with "<name> eq ''" so the user can complete the
      // literal. Preserve anything already in the box by joining with `and`.
      const name = el.dataset.name;
      if (!name) return;
      const input = document.getElementById('qFilter');
      const current = (input.value || '').trim();
      const snippet = `${name} eq ''`;
      input.value = current ? `${current} and ${snippet}` : snippet;
      input.focus();
      // Place caret just inside the trailing quotes so typing fills the value.
      const caret = input.value.lastIndexOf("''") + 1;
      if (caret > 0) input.setSelectionRange(caret, caret);
    } else if (action === 'back-to-services') {
      document.getElementById('serviceInput').value = '';
      const tab = getActiveTab();
      if (tab) tab._serviceInput = '';
      searchServices(lastSearchQuery === '' ? '' : lastSearchQuery);
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
