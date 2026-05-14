// ── Add-profile modal ──
//
// Modal lifecycle (show / hide / mode-switch) plus the save + test
// flows. Save calls `add_profile` with a keyring path; if the keyring
// errors with the `KEYRING_FAILED:` prefix the backend signals, we
// confirm before falling back to plaintext storage. Test runs a
// `test_connection` round-trip with the entered credentials *without*
// persisting them anywhere.
//
// All imports flow downward — no app.js circular.

import { invoke } from './vendor/tauri-core.js';
import { timedInvoke } from './api.js';
import { loadProfiles } from './services.js';
import { state } from './state.js';
import { setStatus } from './status.js';

// Module-level: `null` = add mode, otherwise the profile name being edited.
// Save/test branch on this to drive the blank-password-keeps-keyring semantics
// and the locked-name affordance. Rename is intentionally out of scope.
let editingProfile = null;

function setModalTitle(text) {
  const titleEl = document.querySelector('#addProfileModal h3');
  if (titleEl) titleEl.textContent = text;
}

function setPasswordPlaceholders(editing, authMode) {
  const passInput = document.getElementById('mpPass');
  const passHint  = passInput.nextElementSibling;
  if (editing && authMode === 'basic') {
    passInput.placeholder = 'Leave blank to keep existing';
    if (passHint) passHint.textContent = 'Blank keeps the current keyring entry';
  } else {
    passInput.placeholder = '';
    if (passHint) passHint.textContent = 'Stored in Windows Credential Manager';
  }
}

export function showAddProfileModal() {
  editingProfile = null;
  setModalTitle('Add SAP System');
  document.getElementById('addProfileModal').classList.remove('hidden');
  document.getElementById('mpName').value = '';
  document.getElementById('mpName').disabled = false;
  document.getElementById('mpUser').disabled = false;
  document.getElementById('mpUrl').value = '';
  document.getElementById('mpClient').value = '100';
  document.getElementById('mpLang').value = 'EN';
  document.getElementById('mpAuthMode').value = 'basic';
  document.getElementById('mpUser').value = '';
  document.getElementById('mpPass').value = '';
  document.getElementById('mpAllowSsoDelegate').checked = false;
  updateAuthModeFields();
  setPasswordPlaceholders(false, 'basic');
  document.getElementById('mpError').classList.add('hidden');
  document.getElementById('mpSuccess').classList.add('hidden');
  document.getElementById('mpName').focus();
}

export function showEditProfileModal(profileName) {
  const meta = state.profileMap.get(profileName);
  if (!meta) {
    setStatus(`Profile '${profileName}' not found`);
    return;
  }
  editingProfile = profileName;
  setModalTitle(`Edit '${profileName}'`);
  document.getElementById('addProfileModal').classList.remove('hidden');
  document.getElementById('mpName').value = meta.name;
  document.getElementById('mpName').disabled = true;
  // Username is part of the keyring entry key (sap-odata-explorer:<profile>:<user>).
  // Changing it without also rewriting both the keyring entry AND clearing the
  // orphan creates a saved profile whose recorded username doesn't match any
  // keyring entry — the next request then bails with "no password found".
  // Locking it sidesteps the whole class of problem; delete + re-add is the
  // documented path for changing the user.
  document.getElementById('mpUser').disabled = true;
  document.getElementById('mpUrl').value = meta.base_url || '';
  document.getElementById('mpClient').value = meta.client || '100';
  document.getElementById('mpLang').value = meta.language || 'EN';
  document.getElementById('mpAuthMode').value = meta.auth_mode || 'basic';
  document.getElementById('mpUser').value = meta.username || '';
  document.getElementById('mpPass').value = '';
  document.getElementById('mpAllowSsoDelegate').checked = !!meta.sso_delegate;
  updateAuthModeFields();
  setPasswordPlaceholders(true, meta.auth_mode || 'basic');
  document.getElementById('mpError').classList.add('hidden');
  document.getElementById('mpSuccess').classList.add('hidden');
  document.getElementById('mpUrl').focus();
}

export function updateAuthModeFields() {
  const mode = document.getElementById('mpAuthMode').value;
  document.getElementById('mpCredFields').style.display = mode === 'basic' ? '' : 'none';
  document.getElementById('mpSsoDelegateField').classList.toggle('hidden', mode !== 'sso');

  const hint = document.getElementById('mpAuthHint');
  if (mode === 'sso') {
    hint.textContent = 'Uses Windows integrated auth via Kerberos / Negotiate.';
  } else if (mode === 'browser') {
    hint.textContent = 'Opens an in-app sign-in window for Azure AD / SAP IAS style browser authentication.';
  } else {
    hint.textContent = 'Stores the password in Windows Credential Manager.';
  }
  setPasswordPlaceholders(editingProfile !== null, mode);
}

export function hideAddProfileModal() {
  document.getElementById('addProfileModal').classList.add('hidden');
}

export async function saveProfileModal() {
  const name     = document.getElementById('mpName').value.trim();
  const url      = document.getElementById('mpUrl').value.trim();
  const client   = document.getElementById('mpClient').value.trim();
  const language = document.getElementById('mpLang').value.trim();
  const authMode = document.getElementById('mpAuthMode').value;
  const user     = authMode === 'basic' ? document.getElementById('mpUser').value.trim() : '';
  const pass     = authMode === 'basic' ? document.getElementById('mpPass').value : '';
  const allowSsoDelegate = authMode === 'sso'
    && document.getElementById('mpAllowSsoDelegate').checked;

  const errEl = document.getElementById('mpError');
  const okEl  = document.getElementById('mpSuccess');
  errEl.classList.add('hidden');
  okEl.classList.add('hidden');

  if (!name || !url) {
    errEl.textContent = 'Profile name and URL are required';
    errEl.classList.remove('hidden');
    return;
  }
  if (authMode === 'basic' && !user) {
    // Reachable in edit mode only if the user switched a previously-SSO
    // profile to basic — the username field is locked at its original empty
    // value. Surface a clearer hint than the generic "Username is required".
    errEl.textContent = editingProfile !== null
      ? 'Switching this profile to basic authentication requires a username — delete and re-add the profile instead.'
      : 'Username is required for basic authentication';
    errEl.classList.remove('hidden');
    return;
  }
  // When editing, blank password = keep the existing keyring entry. The
  // backend already skips the keyring write when `password.is_empty()`, so
  // the saved profile keeps pointing at the previously-stored credential.
  if (authMode === 'basic' && !pass && editingProfile === null) {
    errEl.textContent = 'Password is required for basic authentication';
    errEl.classList.remove('hidden');
    return;
  }

  const doSave = async (allowPlaintextFallback) => {
    return await invoke('add_profile', {
      name, baseUrl: url, client, language, authMode, username: user, password: pass,
      allowPlaintextFallback, allowSsoDelegate,
    });
  };

  try {
    let msg;
    try {
      msg = await doSave(false);
    } catch (e) {
      const errStr = String(e);
      // Backend signals keyring failure with a specific prefix so we can offer
      // an explicit confirmation instead of silently downgrading to plaintext.
      if (errStr.includes('KEYRING_FAILED')) {
        const proceed = window.confirm(
          'The OS keyring is unavailable or rejected the password.\n\n' +
          'Store the password in the config file as plaintext instead?\n' +
          '(Not recommended — the file is only protected by your OS file permissions.)'
        );
        if (!proceed) throw e;
        msg = await doSave(true);
      } else {
        throw e;
      }
    }
    okEl.textContent = msg;
    okEl.classList.remove('hidden');
    const wasEditing = editingProfile !== null;
    await loadProfiles();
    if (!wasEditing) {
      // Add flow: auto-switch to the new profile, mirroring pre-edit-mode
      // behaviour. Edit flow keeps whatever the user had selected — the
      // dropdown option text already refreshed via loadProfiles, and any
      // auth-mode change is reflected by updateProfileAuthUi inside it.
      document.getElementById('profileSelect').value = name;
      document.getElementById('profileSelect').dispatchEvent(new Event('change'));
    }
    setTimeout(hideAddProfileModal, 800);
  } catch (e) {
    errEl.textContent = String(e).replace(/^KEYRING_FAILED:\s*/, '');
    errEl.classList.remove('hidden');
  }
}

export async function testProfileModal() {
  const url    = document.getElementById('mpUrl').value.trim();
  const client = document.getElementById('mpClient').value.trim();
  const language = document.getElementById('mpLang').value.trim() || 'EN';
  const authMode = document.getElementById('mpAuthMode').value;
  const user   = authMode === 'basic' ? document.getElementById('mpUser').value.trim() : '';
  const pass   = authMode === 'basic' ? document.getElementById('mpPass').value : '';
  const allowSsoDelegate = authMode === 'sso'
    && document.getElementById('mpAllowSsoDelegate').checked;
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
  // test_connection runs a fresh round-trip with the entered creds — it does
  // not consult the keyring. In edit mode the password field is blank when
  // the user wants to keep the existing entry, so surface that explicitly
  // instead of dispatching a guaranteed 401.
  if (editingProfile !== null && authMode === 'basic' && !pass) {
    errEl.textContent = 'Enter the password to test, or use Save to keep the existing keyring entry.';
    errEl.classList.remove('hidden');
    return;
  }

  try {
    const msg = await timedInvoke('test_connection', {
      baseUrl: url, client, language, authMode, username: user, password: pass,
      allowSsoDelegate,
    });
    okEl.textContent = msg;
    okEl.classList.remove('hidden');
  } catch (e) {
    errEl.textContent = String(e);
    errEl.classList.remove('hidden');
  }
}
