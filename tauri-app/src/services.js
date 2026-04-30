// ── Service catalog + entity browse ──
//
// Wraps the catalog/describe HTTP path: profile dropdown population
// (loadProfiles), full-text search over the service catalog
// (loadService / searchServices / filterServices), service picking +
// resolution (pickService / resolveAndLoadService), and the sidebar
// renderers (renderServiceList / renderFavoritesOnlySidebar /
// renderEntityList). selectEntity drives the describe-panel round-trip
// when the user clicks an entity set.
//
// Cross-module dependencies come back through circular imports for
// renderAnnotationBadge / renderDescribe (still in app.js) and the
// favorites pair (getFavorites/saveFavorites). ESM handles the cycles
// because every reference is inside a function body, not top-level.

import { state } from './state.js';
import { invoke } from './vendor/tauri-core.js';
import { setStatus } from './status.js';
import { safeHtml } from './html.js';
import {
  tabScope,
  timedInvoke,
  updateServicePathBar,
} from './api.js';
import { getActiveTab, renderTabBar } from './tabs.js';
import {
  isBrowserAuthProfile,
  updateProfileAuthUi,
  browserAuthMessage,
} from './auth.js';
import { getFavorites, saveFavorites } from './favorites.js';
import { renderAnnotationBadge, renderDescribe } from './app.js';

export async function loadProfiles() {
  try {
    const profiles = await invoke('get_profiles');
    state.profileMap = new Map(profiles.map(p => [p.name, p]));
    const select = document.getElementById('profileSelect');
    select.innerHTML = '<option value="">Select profile...</option>';
    for (const p of profiles) {
      const opt = document.createElement('option');
      opt.value = p.name;
      opt.textContent = `${p.name} — ${p.base_url.replace('https://', '')}`;
      select.appendChild(opt);
    }
    updateProfileAuthUi(select.value || state.currentProfile);
  } catch (e) {
    setStatus('Error loading profiles: ' + e);
  }
}

export function resetResultsArea() {
  document.getElementById('resultsArea').innerHTML = safeHtml`
    <div class="flex items-center justify-center h-full">
      <div class="text-center">
        <div class="text-ox-amber text-3xl mb-3 opacity-20">&#9670;</div>
        <div class="text-ox-dim text-xs leading-relaxed">
          Select a <span class="text-ox-text">profile</span> &middot; search for a <span class="text-ox-text">service</span> &middot; explore <span class="text-ox-text">entities</span>
        </div>
      </div>
    </div>`;
}

export async function loadService() {
  const input = document.getElementById('serviceInput').value.trim();
  if (!state.currentProfile) { setStatus('Select a profile first'); return; }
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
export function isServicePath(s) {
  return typeof s === 'string' && s.startsWith('/sap/');
}

export async function searchServices(query) {
  if (!state.currentProfile) return;
  const tab = getActiveTab();

  if (tab && tab.cachedServices && tab.lastSearchQuery === (query || '')) {
    renderServiceList(filterServices(tab.cachedServices, query));
    return;
  }

  setStatus(query ? `Searching '${query}'...` : 'Loading catalog...');
  const scope = tabScope();

  try {
    if (!state.cachedServices) {
      const services = await timedInvoke('get_services', {
        profileName: state.currentProfile,
        search: null,
        v4Only: false,
      });
      if (!scope.active()) return;
      state.cachedServices = services;
      if (tab) tab.cachedServices = services;
    }

    state.lastSearchQuery = query || '';
    if (tab) tab.lastSearchQuery = state.lastSearchQuery;

    const filtered = filterServices(state.cachedServices, query);
    renderServiceList(filtered);
    setStatus(`${filtered.length} service(s)${query ? ` matching '${query}'` : ''}`);
  } catch (e) {
    if (!scope.active()) return;
    setStatus('Error: ' + e);
    const message = isBrowserAuthProfile(state.currentProfile) ? browserAuthMessage(e) : String(e);
    document.getElementById('resultsArea').innerHTML =
      safeHtml`<div class="p-4 text-ox-red text-sm">${message}</div>`;
  }
}

export function filterServices(services, query) {
  if (!query) return services;
  const q = query.toLowerCase();
  return services.filter(s =>
    s.technical_name.toLowerCase().includes(q) ||
    s.title.toLowerCase().includes(q) ||
    s.description.toLowerCase().includes(q)
  );
}

export function makeSvcItem(svc, starred) {
  const div = document.createElement('div');
  div.className = 'sidebar-item px-3 py-2 cursor-pointer';
  div.dataset.action = 'pick-service';
  div.dataset.svc = JSON.stringify(svc);
  const badgeClass = svc.version === 'V4' ? 'badge-v4' : 'badge-v2';
  div.innerHTML = safeHtml`
    <div class="flex items-center gap-1.5">
      <span class="text-[9px] px-1 py-px rounded-sm font-mono ${badgeClass}">${svc.version || ''}</span>
      <span class="text-[13px] text-ox-text truncate font-mono flex-1">${svc.technical_name}</span>
      <span class="svc-star${starred ? ' starred' : ''}" data-action="toggle-favorite" data-svc-name="${svc.technical_name}">${starred ? '★' : '☆'}</span>
    </div>
    <div class="text-[11px] text-ox-muted truncate mt-0.5 pl-7">${svc.title || svc.description || ''}</div>
  `;
  return div;
}

// Zero-network sidebar render for when a profile is selected and has favorites
// stored locally. Uses only the data captured in getFavorites — no catalog fetch.
export function renderFavoritesOnlySidebar(profile) {
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
    tab._sidebarHtml = list.innerHTML;
  }
}

export function renderServiceList(services, saveState = true) {
  const tab = getActiveTab();
  const profile = tab ? tab.profile : state.currentProfile;

  if (saveState) {
    state.currentServicePath = null;
    state.currentEntitySet = null;
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
    tab._sidebarHtml = list.innerHTML;
  }
}

export async function pickService(svc) {
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

export async function resolveAndLoadService(input, versionHint) {
  if (!state.currentProfile) return;
  setStatus(`Resolving '${input}'...`);
  const scope = tabScope();

  try {
    let path;
    if (isServicePath(input)) {
      path = input;
    } else {
      path = await timedInvoke('resolve_service', {
        profileName: state.currentProfile,
        service: input,
      });
      if (!scope.active()) return;
    }

    state.currentServicePath = path;
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
      profileName: state.currentProfile,
      servicePath: state.currentServicePath,
    });
    if (!scope.active()) return;

    const entities = response.entity_sets || [];
    const summary = response.annotation_summary || { total: 0, by_namespace: {} };

    state.entitySets = entities;
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
    if (!scope.active()) return;
    setStatus('Error: ' + e);
    const message = isBrowserAuthProfile(state.currentProfile) ? browserAuthMessage(e) : String(e);
    document.getElementById('resultsArea').innerHTML =
      safeHtml`<div class="p-4 text-ox-red text-sm">${message}</div>`;
  }
}

export function renderEntityList(entities) {
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
    div.innerHTML = safeHtml`
      <div class="text-[13px] text-ox-text font-mono">${es.name}</div>
      <div class="text-[10px] text-ox-dim font-mono mt-0.5">${es.keys.join(', ')}</div>
    `;
    // Click handled by document-level delegation (data-action="select-entity")
    list.appendChild(div);
  }

  if (tab) {
    tab._sidebarTitle = 'Entities';
    tab._sidebarCount = String(entities.length);
    tab._sidebarHtml = list.innerHTML;
  }
}

export async function selectEntity(entitySetName, element) {
  document.querySelectorAll('.sidebar-item').forEach(el => el.classList.remove('active'));
  if (element) element.classList.add('active');

  state.currentEntitySet = entitySetName;
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
  const scope = tabScope();
  try {
    const info = await timedInvoke('describe_entity', {
      profileName: state.currentProfile,
      servicePath: state.currentServicePath,
      entitySet: entitySetName,
    });
    if (!scope.active()) return;
    renderDescribe(info);
    setStatus(`${entitySetName} — ${info.properties.length} props, ${info.nav_properties.length} navs`);
  } catch (e) {
    if (!scope.active()) return;
    setStatus('Error: ' + e);
  }
}
