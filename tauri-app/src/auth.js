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
// All imports flow downward — no circular back to app.js.

import { state } from './state.js';
import { invoke } from './vendor/tauri-core.js';
import { setStatus } from './status.js';
import { timedInvoke, updateServicePathBar } from './api.js';
import { safeHtml } from './html.js';
import { getActiveTab } from './tabs.js';
import { resetResultsArea, loadProfiles } from './services.js';
import { clearTraceState } from './trace.js';

export function getProfileMeta(profileName) {
  return profileName ? (state.profileMap.get(profileName) || null) : null;
}

export function isBrowserAuthProfile(profileName = state.currentProfile) {
  return getProfileMeta(profileName)?.auth_mode === 'browser';
}

/// `true` when the active (or named) profile is an offline-mode bucket
/// rather than a live SAP connection. Frontend gating for execute /
/// sign-in / edit buttons reads this to disable affordances that the
/// backend would reject (`assert_network_allowed`).
export function isOfflineProfile(profileName = state.currentProfile) {
  return getProfileMeta(profileName)?.kind === 'offline';
}

export function updateProfileAuthUi(profileName = state.currentProfile) {
  const signInBtn  = document.getElementById('btnProfileSignIn');
  const signOutBtn = document.getElementById('btnProfileSignOut');
  const editBtn    = document.getElementById('btnEditProfile');
  const removeBtn  = document.getElementById('btnRemoveProfile');
  const saveOfflineBtn = document.getElementById('btnSaveOffline');
  const runBtn = document.getElementById('btnRun');

  const offline = isOfflineProfile(profileName);

  // Sign In / Sign Out: only for live browser-SSO profiles. Hidden
  // for connected-basic and for offline (no network identity).
  if (!profileName || offline || !isBrowserAuthProfile(profileName)) {
    signInBtn.classList.add('hidden');
    signOutBtn.classList.add('hidden');
  } else {
    signInBtn.classList.remove('hidden');
    signOutBtn.classList.remove('hidden');
  }

  // Edit: hidden for offline profiles (the connection-form fields
  // — base_url, client, language, etc. — don't apply). Remove
  // remains available so the user can delete an offline bucket.
  if (!profileName) {
    editBtn.classList.add('hidden');
    removeBtn.classList.add('hidden');
  } else if (offline) {
    editBtn.classList.add('hidden');
    removeBtn.classList.remove('hidden');
  } else {
    editBtn.classList.remove('hidden');
    removeBtn.classList.remove('hidden');
  }

  // "Save offline" button (header): visible only for live connected
  // profiles. Hidden when no profile is selected or when the active
  // profile is already an offline bucket.
  if (saveOfflineBtn) {
    if (!profileName || offline) {
      saveOfflineBtn.classList.add('hidden');
    } else {
      saveOfflineBtn.classList.remove('hidden');
    }
  }

  // Run / query-execute button: disabled when active profile is
  // offline. The backend already fails-closed via
  // `assert_network_allowed` on `run_query`; this is defense-in-depth
  // at the UI layer so the user doesn't see a "no network allowed"
  // error after clicking — the button just isn't actionable.
  if (runBtn) {
    if (offline) {
      runBtn.setAttribute('disabled', '');
      runBtn.classList.add('opacity-40', 'cursor-not-allowed');
      runBtn.title = 'Query execution is disabled for offline profiles';
    } else {
      runBtn.removeAttribute('disabled');
      runBtn.classList.remove('opacity-40', 'cursor-not-allowed');
      runBtn.title = '';
    }
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
