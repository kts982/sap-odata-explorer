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
import { setStatus } from './status.js';
import { loadProfiles } from './services.js';
import { isOfflineProfile } from './auth.js';

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

  if (saveBtn) {
    saveBtn.addEventListener('click', saveCurrentServiceOffline);
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

  // Dismiss on backdrop click (matches the existing addProfile modal pattern).
  if (modal) {
    modal.addEventListener('click', (e) => {
      if (e.target === modal) closeImportModal();
    });
  }
}

/// Save-for-offline: call save_service_offline against the active
/// connected profile + currently-loaded service. The user gets a
/// status-bar confirmation; on success we reload profiles (a new
/// `<NAME> (offline)` bucket may have been created).
async function saveCurrentServiceOffline() {
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
  setStatus(`Saving ${state.currentServicePath} from '${state.currentProfile}' offline...`);
  try {
    const outcome = await invoke('save_service_offline', {
      connectedProfileName: state.currentProfile,
      servicePath: state.currentServicePath,
      offlineProfileName: null,
      labelOverride: null,
      note: null,
    });
    const summary = summarizeOutcome(outcome);
    setStatus(`Offline save: ${summary}`);
    // A new offline bucket may have been created; refresh the picker
    // so the user can pick it.
    await loadProfiles();
  } catch (e) {
    setStatus('Save offline failed: ' + e);
  }
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
  await populateImportProfileDropdown();
  modal.classList.remove('hidden');
}

function closeImportModal() {
  const modal = document.getElementById('importEdmxModal');
  if (modal) modal.classList.add('hidden');
  importFile = null;
}

/// Populate the target-offline-profile dropdown with the user's
/// existing offline buckets (plus a default "Imported" entry).
async function populateImportProfileDropdown() {
  const sel = document.getElementById('impProfile');
  sel.innerHTML = '<option value="">Imported (default)</option>';
  try {
    const profiles = await invoke('get_profiles');
    for (const p of profiles) {
      if (p.kind !== 'offline') continue;
      if (p.name === 'Imported') continue; // already the default
      const opt = document.createElement('option');
      opt.value = p.name;
      opt.textContent = p.name;
      sel.appendChild(opt);
    }
  } catch (e) {
    // Non-fatal: if get_profiles fails, the modal still works — the
    // user can submit and Tauri will surface a friendlier error.
    console.warn('Failed to load offline profiles for import dropdown:', e);
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
    if (input.files && input.files.length > 0) {
      importFile = input.files[0];
      document.getElementById('impFilePath').value = importFile.name;
      // Hide stale error/success state from a previous attempt.
      document.getElementById('impError').classList.add('hidden');
      document.getElementById('impSuccess').classList.add('hidden');
    }
  });
  input.click();
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
  const labelOverride = document.getElementById('impLabel').value.trim() || null;
  const note = document.getElementById('impNote').value.trim() || null;
  const targetProfile = document.getElementById('impProfile').value.trim() || null;

  try {
    // The browser sandbox lets us read the bytes; we ship them to
    // Tauri as a Uint8Array. The Rust side enforces the 10 MB cap +
    // every validation step regardless of what JS sent.
    const buf = await importFile.arrayBuffer();
    const bytes = Array.from(new Uint8Array(buf));
    const outcome = await invoke('import_edmx_bytes', {
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
    // Auto-close after a brief moment so the user sees the success line.
    setTimeout(() => closeImportModal(), 1200);
  } catch (e) {
    errEl.textContent = String(e);
    errEl.classList.remove('hidden');
  }
}
