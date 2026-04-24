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
      verEl.className = 'text-[10px] px-1 py-px rounded font-mono ' +
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
  if (input.startsWith('/')) {
    await resolveAndLoadService(input);
  } else {
    await searchServices(input);
  }
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
      <span class="text-[9px] px-1 py-px rounded font-mono ${badgeClass}">${escapeHtml(svc.version || '')}</span>
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
    if (input.startsWith('/')) {
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
    const entities = await timedInvoke('get_entities', {
      profileName: currentProfile,
      servicePath: currentServicePath,
    });

    entitySets = entities;
    if (tab) tab.entitySets = entities;
    renderEntityList(entities);
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

function renderDescribe(info) {
  const panel = document.getElementById('describePanel');
  panel.classList.remove('hidden');
  document.getElementById('entityTitle').textContent = `${info.name}`;

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
    html += `<tr class="hover:bg-ox-amberGlow cursor-pointer transition-colors" data-action="select" data-field="${escapeHtml(p.name)}">`;
    html += `<td class="py-0.5 pr-3 text-ox-text">${escapeHtml(p.name)}</td>`;
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

  const allCols = [...scalarCols, ...nestedCols];

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
        html += `<td class="px-3 py-1"><span class="expand-badge text-[10px] px-1.5 py-0.5 rounded font-mono inline-block" data-action="nested" data-key="${storeKey}" data-col="${escapeHtml(col)}">${count} item${count !== 1 ? 's' : ''}</span></td>`;
      } else if (typeof val === 'object') {
        const storeKey = `r${i}_${col}`;
        expandedDataStore[storeKey] = val;
        html += `<td class="px-3 py-1"><span class="expand-badge text-[10px] px-1.5 py-0.5 rounded font-mono inline-block" data-action="nested" data-key="${storeKey}" data-col="${escapeHtml(col)}">object</span></td>`;
      } else {
        const text = String(val);
        // Feature 7: data-cell-col / data-cell-val for filter tooltip
        html += `<td class="px-3 py-1 text-ox-text whitespace-nowrap cursor-pointer" data-action="cell-click" data-cell-col="${escapeHtml(col)}" data-cell-val="${escapeHtml(text)}">${escapeHtml(text)}</td>`;
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
      <button data-action="close-nested" class="text-ox-dim hover:text-ox-text text-xs px-2 py-0.5 rounded hover:bg-ox-hover">close</button>
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
