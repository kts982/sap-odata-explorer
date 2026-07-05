// ── Offline mode UI ──
//
// The offline-library frontend surface:
//
//   - "Save offline" button (header, visible when a connected profile
//     + service are loaded): captures the live service's $metadata into
//     the offline library under `<connected> (offline)` by default.
//   - "Import EDMX" button + modal (header, always visible): user picks
//     a file from disk; we read the bytes in JS and ship them to the
//     `import_edmx_bytes` Tauri command. No native file-picker plugin
//     dependency — the browser's `<input type="file">` is sufficient
//     because all path-traversal / size / validation concerns are
//     enforced by the Rust side after the bytes arrive.
//
// Auth-side gating (badge in the profile picker, hidden Sign In /
// Edit buttons for offline profiles, disabled Run button) lives in
// `auth.js::updateProfileAuthUi` because it shares state with the
// existing profile-switch hooks.

import { state } from './state.js';
import { invoke } from './vendor/tauri-core.js';
import { timedInvoke } from './api.js';
import { setStatus } from './status.js';
import { loadProfiles, searchServices } from './services.js';
import { getActiveTab } from './tabs.js';
import { isOfflineProfile } from './auth.js';

// Mirror of the Rust side's `MAX_IMPORT_SIZE_BYTES`. Kept here so the
// JS preflight can reject before `arrayBuffer()` is called — otherwise
// the webview pulls the entire file into memory just for Rust to
// reject it, which can freeze the UI on a multi-GB selection. The
// backend remains the source of truth; if these numbers ever diverge,
// the backend's stricter rejection still applies.
const MAX_IMPORT_SIZE_BYTES = 10 * 1024 * 1024;

// Hidden file input used for the Import-EDMX modal's Browse button.
// One-shot per click — the file is read immediately, the input value
// is reset so picking the same file twice still triggers `change`.
let importFile = null;

/// Wire the offline-mode buttons + modal handlers. Called once during
/// app boot (from app.js) after the DOM is ready.
export function wireOfflineButtons() {
  const saveBtn = document.getElementById('btnSaveOffline');
  const importBtn = document.getElementById('btnImportEdmx');
  const modal = document.getElementById('importEdmxModal');
  const closeBtn = document.getElementById('btnImportEdmxClose');
  const cancelBtn = document.getElementById('btnImpCancel');
  const browseBtn = document.getElementById('btnImpBrowse');
  const saveImportBtn = document.getElementById('btnImpSave');
  const soModal = document.getElementById('saveOfflineModal');
  const soCloseBtn = document.getElementById('btnSaveOfflineClose');
  const soCancelBtn = document.getElementById('btnSoCancel');
  const soSubmitBtn = document.getElementById('btnSoSave');

  if (saveBtn) {
    saveBtn.addEventListener('click', openSaveOfflineModal);
  }
  if (importBtn) {
    importBtn.addEventListener('click', openImportModal);
  }
  if (closeBtn) {
    closeBtn.addEventListener('click', closeImportModal);
  }
  if (cancelBtn) {
    cancelBtn.addEventListener('click', closeImportModal);
  }
  if (browseBtn) {
    browseBtn.addEventListener('click', triggerBrowse);
  }
  if (saveImportBtn) {
    saveImportBtn.addEventListener('click', performImport);
  }
  if (soCloseBtn) {
    soCloseBtn.addEventListener('click', closeSaveOfflineModal);
  }
  if (soCancelBtn) {
    soCancelBtn.addEventListener('click', closeSaveOfflineModal);
  }
  if (soSubmitBtn) {
    soSubmitBtn.addEventListener('click', performSaveOffline);
  }

  // Dismiss on backdrop click (matches the existing addProfile modal pattern).
  if (modal) {
    modal.addEventListener('click', (e) => {
      if (e.target === modal) closeImportModal();
    });
  }
  if (soModal) {
    soModal.addEventListener('click', (e) => {
      if (e.target === soModal) closeSaveOfflineModal();
    });
  }
}

/// Open the save-offline modal for the active connected profile +
/// currently-loaded service. The modal lets the user override the
/// target bucket / label / note before the capture; submitting with
/// everything blank behaves exactly like the old one-click save
/// (backend defaults: bucket `<connected> (offline)`, label derived
/// from the Schema Namespace).
async function openSaveOfflineModal() {
  if (!state.currentProfile) {
    setStatus('Select a profile first');
    return;
  }
  if (isOfflineProfile(state.currentProfile)) {
    setStatus('Active profile is already offline — nothing to save');
    return;
  }
  if (!state.currentServicePath) {
    setStatus('Load a service first');
    return;
  }
  const modal = document.getElementById('saveOfflineModal');
  if (!modal) return;
  document.getElementById('soServicePath').textContent = state.currentServicePath;
  document.getElementById('soLabel').value = '';
  document.getElementById('soNote').value = '';
  document.getElementById('soError').classList.add('hidden');
  document.getElementById('soSuccess').classList.add('hidden');
  const defaultBucket = `${state.currentProfile} (offline)`;
  await populateOfflineBucketDropdown('soProfile', `${defaultBucket} (default)`, defaultBucket);
  modal.classList.remove('hidden');
}

function closeSaveOfflineModal() {
  const modal = document.getElementById('saveOfflineModal');
  if (modal) modal.classList.add('hidden');
}

/// Submit the save-offline modal: call save_service_offline with the
/// collected overrides (empty fields → null → backend defaults).
async function performSaveOffline() {
  const errEl = document.getElementById('soError');
  const okEl = document.getElementById('soSuccess');
  errEl.classList.add('hidden');
  okEl.classList.add('hidden');

  const labelOverride = document.getElementById('soLabel').value.trim() || null;
  const note = document.getElementById('soNote').value.trim() || null;
  const targetProfile = document.getElementById('soProfile').value.trim() || null;

  setStatus(`Saving ${state.currentServicePath} from '${state.currentProfile}' offline...`);
  try {
    // `save_service_offline` touches the network (fetch_metadata_xml)
    // and returns the wrapped `{ data, trace }` shape. Route through
    // `timedInvoke` so the trace lands in the active tab's HTTP
    // inspector and we get the spinner + the unwrapped SaveOutcome.
    const outcome = await timedInvoke('save_service_offline', {
      connectedProfileName: state.currentProfile,
      servicePath: state.currentServicePath,
      offlineProfileName: targetProfile,
      labelOverride,
      note,
    });
    const summary = summarizeOutcome(outcome);
    okEl.textContent = `Saved: ${summary}`;
    okEl.classList.remove('hidden');
    setStatus(`Offline save: ${summary}`);
    // A new offline bucket may have been created; refresh the picker
    // so the user can pick it.
    await loadProfiles();
    // If the user happens to be viewing that bucket already (rare for
    // path A — they'd typically be on the connected profile — but
    // possible across tabs), refresh the sidebar so the new row shows.
    await refreshServicesIfActiveProfile(outcome.offline_profile_name);
    // Auto-close after a brief moment so the user sees the success line.
    setTimeout(() => closeSaveOfflineModal(), 1200);
  } catch (e) {
    errEl.textContent = String(e);
    errEl.classList.remove('hidden');
    setStatus('Save offline failed: ' + e);
  }
}

/// Delete one cached service from the active offline bucket. Wired to
/// the per-row `✕` in the sidebar (app.js click delegation). The GUI
/// parity half of CLI `offline delete --service-id`; the bucket itself
/// is kept — removing the whole bucket stays on the profile Remove
/// button.
export async function deleteOfflineServiceRow(serviceId) {
  const profile = state.currentProfile;
  if (!profile || !serviceId || !isOfflineProfile(profile)) return;
  const ok = confirm(
    `Delete '${serviceId}' from offline profile '${profile}'?\n\n` +
    'The cached EDMX file is removed from disk. The bucket itself is kept.'
  );
  if (!ok) return;
  try {
    const msg = await invoke('delete_offline_service', {
      profileName: profile,
      serviceId,
    });
    setStatus(String(msg));
    await refreshServicesIfActiveProfile(profile);
  } catch (e) {
    setStatus('Delete failed: ' + e);
  }
}

/// If the just-mutated offline bucket is the user's currently-active
/// profile, the cached services list (sidebar) needs to be refreshed
/// so the new / updated row shows up immediately. Without this, the
/// `change`-event-driven cache survives across import / save calls
/// and the user has to restart the app (or switch profiles and back)
/// to see the new entry. Bug discovered in 2026-05-19 smoke testing.
async function refreshServicesIfActiveProfile(outcomeProfileName) {
  if (!outcomeProfileName) return;
  if (state.currentProfile !== outcomeProfileName) return;
  // Bust both caches: the global one used by `searchServices` for its
  // single-shot dedupe, and the per-tab one used to render the sidebar
  // after a tab-switch. Mirrors the profile-change reset in app.js.
  state.cachedServices = null;
  state.lastSearchQuery = null;
  const tab = getActiveTab();
  if (tab) {
    tab.cachedServices = null;
    tab.lastSearchQuery = null;
  }
  await searchServices('');
}

function summarizeOutcome(outcome) {
  // `kind` is serialized snake_case from the Rust enum.
  const kindLabel = ({
    new_service: 'new entry',
    overwrite_updated_bytes: 'updated existing entry',
    skipped_byte_identical: 'no change (bytes identical)',
  })[outcome.kind] || outcome.kind;
  const bucket = outcome.created_new_offline_profile
    ? ` (created bucket "${outcome.offline_profile_name}")`
    : ` → "${outcome.offline_profile_name}"`;
  return `${kindLabel}${bucket}`;
}

async function openImportModal() {
  const modal = document.getElementById('importEdmxModal');
  if (!modal) return;
  importFile = null;
  document.getElementById('impFilePath').value = '';
  document.getElementById('impLabel').value = '';
  document.getElementById('impNote').value = '';
  document.getElementById('impError').classList.add('hidden');
  document.getElementById('impSuccess').classList.add('hidden');
  await populateOfflineBucketDropdown('impProfile', 'Imported (default)', 'Imported');
  modal.classList.remove('hidden');
}

function closeImportModal() {
  const modal = document.getElementById('importEdmxModal');
  if (modal) modal.classList.add('hidden');
  importFile = null;
}

/// Populate a target-offline-profile dropdown with the user's existing
/// offline buckets. The first entry is always the given default (empty
/// value → backend picks its own default bucket); `excludeName` skips
/// the bucket the default already represents so it isn't listed twice.
/// Shared by the Import-EDMX and Save-offline modals.
async function populateOfflineBucketDropdown(selectId, defaultLabel, excludeName) {
  const sel = document.getElementById(selectId);
  sel.innerHTML = '';
  const def = document.createElement('option');
  def.value = '';
  def.textContent = defaultLabel;
  sel.appendChild(def);
  try {
    const profiles = await invoke('get_profiles');
    for (const p of profiles) {
      if (p.kind !== 'offline') continue;
      if (p.name === excludeName) continue; // already the default
      const opt = document.createElement('option');
      opt.value = p.name;
      opt.textContent = p.name;
      sel.appendChild(opt);
    }
  } catch (e) {
    // Non-fatal: if get_profiles fails, the modal still works — the
    // user can submit and Tauri will surface a friendlier error.
    console.warn('Failed to load offline profiles for dropdown:', e);
  }
}

/// Trigger the browser's file picker via a one-shot hidden `<input
/// type="file">`. On selection, store the File object in the
/// `importFile` closure variable and show its name in the readonly
/// path input.
function triggerBrowse() {
  const input = document.createElement('input');
  input.type = 'file';
  input.accept = '.edmx,.xml,application/xml,text/xml';
  input.addEventListener('change', () => {
    if (!input.files || input.files.length === 0) return;
    const picked = input.files[0];
    const errEl = document.getElementById('impError');
    const okEl = document.getElementById('impSuccess');
    errEl.classList.add('hidden');
    okEl.classList.add('hidden');

    // **Size preflight on the File object, before `arrayBuffer()`.**
    // The browser exposes `File.size` cheaply (from the directory
    // entry, no read), so we can reject huge selections without
    // pulling them into memory. Without this, picking a 5 GB file
    // would freeze the webview while it slurped the whole thing
    // just for the Rust side to reject it.
    if (picked.size > MAX_IMPORT_SIZE_BYTES) {
      importFile = null;
      document.getElementById('impFilePath').value = '';
      errEl.textContent =
        `File is ${formatBytes(picked.size)}; the offline-import cap is ${formatBytes(MAX_IMPORT_SIZE_BYTES)}. Real $metadata is rarely above 2 MB — this is probably the wrong file.`;
      errEl.classList.remove('hidden');
      return;
    }

    importFile = picked;
    document.getElementById('impFilePath').value = picked.name;
  });
  input.click();
}

function formatBytes(n) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

async function performImport() {
  const errEl = document.getElementById('impError');
  const okEl = document.getElementById('impSuccess');
  errEl.classList.add('hidden');
  okEl.classList.add('hidden');

  if (!importFile) {
    errEl.textContent = 'Pick a file first.';
    errEl.classList.remove('hidden');
    return;
  }
  // Re-check size at submit time. The picker already preflighted, but
  // the File reference is from a one-shot input; in theory the
  // underlying file could have been swapped between Browse and Import.
  // Cheap insurance.
  if (importFile.size > MAX_IMPORT_SIZE_BYTES) {
    errEl.textContent =
      `File is ${formatBytes(importFile.size)}; the offline-import cap is ${formatBytes(MAX_IMPORT_SIZE_BYTES)}.`;
    errEl.classList.remove('hidden');
    return;
  }
  const labelOverride = document.getElementById('impLabel').value.trim() || null;
  const note = document.getElementById('impNote').value.trim() || null;
  const targetProfile = document.getElementById('impProfile').value.trim() || null;

  try {
    // The browser sandbox lets us read the bytes; we ship them to
    // Tauri as a Uint8Array. The Rust side enforces the 10 MB cap +
    // every validation step regardless of what JS sent.
    const buf = await importFile.arrayBuffer();
    const bytes = Array.from(new Uint8Array(buf));
    // `import_edmx_bytes` returns `SaveOutcome` directly (no trace —
    // path B doesn't touch the network). `timedInvoke` is a
    // pass-through for that shape but it still drives the global
    // spinner, which is a noticeable UX win on multi-MB imports
    // because the bytes-upload + atomic write can take a moment.
    const outcome = await timedInvoke('import_edmx_bytes', {
      bytes,
      originalFilename: importFile.name,
      targetOfflineProfile: targetProfile,
      labelOverride,
      note,
    });
    const summary = summarizeOutcome(outcome);
    okEl.textContent = `Imported: ${summary}`;
    okEl.classList.remove('hidden');
    setStatus(`Imported '${importFile.name}': ${summary}`);
    // Refresh the picker so the user can select the (possibly new) bucket.
    await loadProfiles();
    // If the user was already viewing that bucket, the sidebar would
    // otherwise survive on its cached services list (no `change`
    // event fired because the picker value didn't move) — bust the
    // cache and re-fetch so the freshly-imported row appears.
    await refreshServicesIfActiveProfile(outcome.offline_profile_name);
    // Auto-close after a brief moment so the user sees the success line.
    setTimeout(() => closeImportModal(), 1200);
  } catch (e) {
    errEl.textContent = String(e);
    errEl.classList.remove('hidden');
  }
}
