// ── App version, update check, feedback ──
//
// The status bar shows the app version. Clicking it asks GitHub for the
// latest release — user-initiated only; the app never phones home on its
// own. If a newer release exists, the next click opens its page.
// "Feedback" opens the project's new-issue chooser. Both pages are fixed
// on the Rust side (`open_project_page`); the webview only names them.
//
// All imports flow downward — no circular back to app.js.

import { invoke } from './vendor/tauri-core.js';
import { setStatus } from './status.js';
import { timedInvoke } from './api.js';

let newerRelease = null; // set once a check finds a newer release

export async function initAppInfo() {
  const btn = document.getElementById('btnAppVersion');
  try {
    const info = await invoke('app_info');
    btn.textContent = `v${info.version}`;
  } catch {
    btn.textContent = '';
  }
}

export async function onVersionClick() {
  if (newerRelease) {
    await openProjectPage('releases');
    return;
  }
  setStatus('Checking for updates...');
  try {
    const update = await timedInvoke('check_for_update');
    if (update.newer) {
      newerRelease = update.latest;
      const btn = document.getElementById('btnAppVersion');
      btn.textContent = `v${update.current} → v${update.latest}`;
      btn.title = 'Open the release page';
      btn.classList.add('text-ox-amber');
      setStatus(`Version ${update.latest} is available — click the version in the status bar to open the release page`);
    } else {
      setStatus(`Up to date — latest release is v${update.latest}`);
    }
  } catch (e) {
    setStatus(String(e));
  }
}

export async function openProjectPage(page) {
  try {
    await invoke('open_project_page', { page });
  } catch (e) {
    setStatus(String(e));
  }
}
