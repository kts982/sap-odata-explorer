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
