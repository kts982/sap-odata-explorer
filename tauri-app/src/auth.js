// ── Profile auth ──
//
// Profile metadata + sign-in / sign-out / remove flows for the active
// profile. Two auth modes are observable from the desktop UI:
//   - Basic auth (default): keyring-backed password; no sign-in flow.
//   - Browser SSO (`auth_mode === 'browser'`): the user signs in via
//     a popup webview; the session cookie is captured by the Tauri
//     side and replayed on each request. The Sign In / Sign Out
//     buttons next to the profile selector only show for these.
//
// `removeCurrentProfile` and `signOutCurrentProfile` reach back into
// app.js for `clearTraceState`, `resetResultsArea`, `loadProfiles` —
// circular import. ESM resolves the circle because all references sit
// inside function bodies.

import { state } from './state.js';
import { invoke } from './vendor/tauri-core.js';
import { setStatus } from './status.js';
import { timedInvoke, updateServicePathBar } from './api.js';
import { safeHtml } from './html.js';
import { getActiveTab } from './tabs.js';
import { clearTraceState, resetResultsArea, loadProfiles } from './app.js';

export function getProfileMeta(profileName) {
  return profileName ? (state.profileMap.get(profileName) || null) : null;
}

export function isBrowserAuthProfile(profileName = state.currentProfile) {
  return getProfileMeta(profileName)?.auth_mode === 'browser';
}

export function updateProfileAuthUi(profileName = state.currentProfile) {
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

export async function signOutCurrentProfile() {
  if (!state.currentProfile) { setStatus('Select a profile first'); return; }
  if (!isBrowserAuthProfile(state.currentProfile)) {
    setStatus('Sign Out only applies to browser SSO profiles');
    return;
  }
  try {
    const msg = await invoke('sign_out_profile', { profileName: state.currentProfile });
    clearTraceState(getActiveTab());
    setStatus(msg);
  } catch (e) {
    setStatus('Sign out failed: ' + e);
  }
}

export async function removeCurrentProfile() {
  if (!state.currentProfile) {
    setStatus('Select a profile first');
    return;
  }
  const name = state.currentProfile;
  if (!confirm(`Remove profile '${name}'?\n\nThis will also delete its password from the OS keyring.`)) {
    return;
  }
  try {
    const msg = await invoke('remove_profile', { name });
    setStatus(msg);
    clearTraceState(getActiveTab());
    // Reset UI state
    state.currentProfile = null;
    state.cachedServices = null;
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

export function browserAuthMessage(err) {
  return `${String(err)}\n\nBrowser SSO session required. Use the Sign In button next to the profile selector.`;
}

export async function signInCurrentProfile() {
  if (!state.currentProfile) {
    setStatus('Select a profile first');
    return;
  }
  if (!isBrowserAuthProfile(state.currentProfile)) {
    setStatus('The selected profile does not use browser SSO');
    return;
  }

  setStatus(`Signing in to ${state.currentProfile}...`);
  try {
    const msg = await timedInvoke('browser_sign_in_profile', { profileName: state.currentProfile });
    setStatus(msg);
  } catch (e) {
    setStatus('Sign-in failed: ' + e);
    document.getElementById('resultsArea').innerHTML =
      safeHtml`<div class="p-4 text-ox-red text-sm">${String(e)}</div>`;
  }
}
