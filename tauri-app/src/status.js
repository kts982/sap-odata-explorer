// ── Status bar + global spinner ──
// Thin DOM helpers for the bottom status bar (left-side message, right-side
// elapsed-ms) and the top-of-screen progress spinner shown while invokes
// are in flight.

export function setStatus(msg) {
  document.getElementById('statusText').textContent = msg;
}

export function setTime(ms) {
  document.getElementById('statusTime').textContent = ms ? `${ms}ms` : '';
}

export function showSpinner() {
  document.getElementById('globalSpinner').classList.remove('hidden');
}

export function hideSpinner() {
  document.getElementById('globalSpinner').classList.add('hidden');
}
