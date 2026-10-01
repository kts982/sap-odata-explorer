// ── Per-profile favorites ──
//
// Favorites are stored in localStorage under `ox_favorites_<profileName>`,
// keyed per profile so different SAP systems show their own starred set.
//
// Legacy entries were arrays of `technical_name` strings; the current
// shape is the full service object `{ technical_name, title, description,
// service_url, version }`. `getFavorites` normalises legacy strings to
// stub objects on read; `renderServiceList` upgrades stubs to full
// objects when the catalog fetch returns the real data.
//
// Imports filterServices / renderServiceList / renderFavoritesOnlySidebar
// from services.js so toggling a star can immediately re-render the
// sidebar. That makes favorites.js <-> services.js a circular pair; ESM
// resolves it because the bindings are only read inside function bodies.

import { state } from './state.js';
import { getActiveTab } from './tabs.js';
import {
  filterServices,
  renderServiceList,
  renderFavoritesOnlySidebar,
} from './services.js';

export function favKey(profileName) {
  return `ox_favorites_${profileName}`;
}

export function getFavorites(profileName) {
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

export function saveFavorites(profileName, list) {
  localStorage.setItem(favKey(profileName), JSON.stringify(list));
}

export function favIndex(favs, svcName) {
  return favs.findIndex(f => f.technical_name === svcName);
}

export function isFavorite(profileName, svcName) {
  return favIndex(getFavorites(profileName), svcName) !== -1;
}

// Readable name for a pasted service path: the service segment before a
// numeric version (V4 `/…/srvd/sap/<service>/0001`), else the last one.
export function serviceNameFromPath(path) {
  const parts = String(path || '').split('/').filter(Boolean);
  if (parts.length === 0) return '';
  const last = parts[parts.length - 1];
  const name = /^\d+$/.test(last) && parts.length > 1 ? parts[parts.length - 2] : last;
  return name.split(';')[0].toUpperCase();
}

function favPathIndex(favs, path) {
  return favs.findIndex(f => f.service_url && f.service_url === path);
}

export function isFavoritePath(profileName, path) {
  return !!profileName && !!path && favPathIndex(getFavorites(profileName), path) !== -1;
}

// Star the service loaded in the active tab — including one opened by
// pasting its path (customers without the V4 catalog published have no
// catalog entry to star).
export function toggleFavoriteCurrentService() {
  const tab = getActiveTab();
  if (!tab || !tab.profile || !tab.servicePath) return;
  const favs = getFavorites(tab.profile);
  const idx = favPathIndex(favs, tab.servicePath);
  if (idx === -1) {
    const name = serviceNameFromPath(tab.servicePath);
    favs.push({
      technical_name: name,
      title: name,
      description: tab.servicePath,
      service_url: tab.servicePath,
      version: tab.serviceVersion || '',
    });
  } else {
    favs.splice(idx, 1);
  }
  saveFavorites(tab.profile, favs);
  updateFavPathStar(tab);
}

export function updateFavPathStar(tab) {
  const btn = document.getElementById('btnFavPath');
  if (!btn) return;
  const starred = !!tab && isFavoritePath(tab.profile, tab.servicePath);
  btn.textContent = starred ? '★' : '☆';
  btn.classList.toggle('text-ox-amber', starred);
  btn.title = starred ? 'Remove from favorites' : 'Favorite this service — it then shows under Favorites for this profile';
}

export function toggleFavorite(svc, starEl) {
  const tab = getActiveTab();
  const profile = tab ? tab.profile : state.currentProfile;
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
