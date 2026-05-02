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

export function showAddProfileModal() {
  document.getElementById('addProfileModal').classList.remove('hidden');
  document.getElementById('mpName').value = '';
  document.getElementById('mpUrl').value = '';
  document.getElementById('mpClient').value = '100';
  document.getElementById('mpLang').value = 'EN';
  document.getElementById('mpAuthMode').value = 'basic';
  document.getElementById('mpUser').value = '';
  document.getElementById('mpPass').value = '';
  document.getElementById('mpAllowSsoDelegate').checked = false;
  updateAuthModeFields();
  document.getElementById('mpError').classList.add('hidden');
  document.getElementById('mpSuccess').classList.add('hidden');
  document.getElementById('mpName').focus();
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
  if (authMode === 'basic' && (!user || !pass)) {
    errEl.textContent = 'Username and password are required for basic authentication';
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
    await loadProfiles();
    document.getElementById('profileSelect').value = name;
    document.getElementById('profileSelect').dispatchEvent(new Event('change'));
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
