// ── Clipboard helper ──
// Thin wrapper around `navigator.clipboard.writeText` that surfaces
// success / failure through the status bar. Used by the trace
// inspector (curl / body copy) and results renderer (column / row /
// URL copy).

import { setStatus } from './status.js';

export async function copyToClipboard(text, label) {
  try {
    await navigator.clipboard.writeText(text);
    setStatus(`Copied ${label || 'to clipboard'}`);
  } catch (e) {
    setStatus('Copy failed: ' + e);
  }
}
