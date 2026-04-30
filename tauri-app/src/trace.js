// ── HTTP trace inspector ──
//
// Two-pane viewer for the per-tab `tab.httpTraceEntries` list — left
// pane is the request list, right pane is the selected request's
// detail with Request/Response sub-tabs. The detail view supports
// "show full body" expansion for long bodies and "copy as curl" /
// copy-body shortcuts.
//
// State lives on the active tab (`httpTraceEntries`, `selectedTraceId`,
// `_traceVisible`, `_traceSubTab`, `_traceBodyExpanded`); no
// module-private mutables here. That keeps trace state
// per-tab-correlated, which matters for the SAP-request flow where a
// long-running query can complete after the user has switched to a
// different tab.
//
// No circular imports back to app.js. The 5c batch 6b extraction
// closed the api.js → app.js and auth.js → app.js circles by giving
// them clean trace.js targets.

import { state } from './state.js';
import { safeHtml, raw } from './html.js';
import { setStatus } from './status.js';
import { getActiveTab } from './tabs.js';
import { copyToClipboard } from './clipboard.js';

export function clearTraceState(tab = getActiveTab()) {
  if (!tab) return;
  tab.httpTraceEntries = [];
  tab.selectedTraceId = null;
  tab._traceVisible = false;
  if (tab.id === state.activeTabId) {
    document.getElementById('traceInspectorPanel').classList.add('hidden');
    renderTraceSummary(tab);
  }
}

export function ensureTraceSelection(tab) {
  if (!tab) return null;
  if (
    tab.selectedTraceId &&
    tab.httpTraceEntries.some(entry => entry.id === tab.selectedTraceId)
  ) {
    return tab.selectedTraceId;
  }
  tab.selectedTraceId = tab.httpTraceEntries.length
    ? tab.httpTraceEntries[tab.httpTraceEntries.length - 1].id
    : null;
  return tab.selectedTraceId;
}

function getSelectedTraceEntry(tab = getActiveTab()) {
  if (!tab) return null;
  const selectedId = ensureTraceSelection(tab);
  if (!selectedId) return null;
  return tab.httpTraceEntries.find(entry => entry.id === selectedId) || null;
}

export function renderTraceSummary(tab = getActiveTab()) {
  const count = tab?.httpTraceEntries?.length || 0;
  document.getElementById('traceSummary').textContent = count
    ? `${count} request${count === 1 ? '' : 's'}`
    : 'No trace';
  document.getElementById('traceCount').textContent = count
    ? `${count} request${count === 1 ? '' : 's'}`
    : 'No requests';
}

function traceStatusClass(entry) {
  if (entry.error) return 'err';
  if (!entry.status) return '';
  if (entry.status >= 500) return 'err';
  if (entry.status >= 400) return 'warn';
  if (entry.status >= 300) return 'warn';
  return 'ok';
}

function traceStatusLabel(entry) {
  if (entry.status) return String(entry.status);
  if (entry.error) return 'ERR';
  return 'OPEN';
}

function compactTraceUrl(url) {
  try {
    const parsed = new URL(url);
    return `${parsed.host}${parsed.pathname}${parsed.search}`;
  } catch {
    return url;
  }
}

function traceOutcomeLabel(entry) {
  if (entry.error) return entry.error;
  if (entry.redirect_location) return `redirect → ${entry.redirect_location}`;
  return entry.response_body_preview ? 'response captured' : 'headers captured';
}

function renderTraceHeaders(headers) {
  if (!headers || headers.length === 0) {
    return '<div class="trace-code text-ox-dim">No headers captured.</div>';
  }

  const rows = headers
    .map(header => safeHtml`
      <div class="trace-header-name">${header.name}</div>
      <div class="trace-header-value">${header.value}</div>`)
    .join('');
  return safeHtml`<div class="trace-header-grid">${raw(rows)}</div>`;
}

// First-render length for trace bodies — keeps the inspector snappy on
// large responses. The "show full" button below the preview reveals the
// rest (up to the core's MAX_BODY_PREVIEW_CHARS cap — anything beyond
// that arrives with a `... <truncated>` suffix already in place).
const TRACE_BODY_PREVIEW_CHARS = 4000;

function renderTraceBody(body, emptyLabel) {
  if (!body) {
    return safeHtml`<div class="trace-code text-ox-dim">${emptyLabel}</div>`;
  }
  if (body.length <= TRACE_BODY_PREVIEW_CHARS) {
    return safeHtml`<pre class="trace-code">${body}</pre>`;
  }
  // Long body — render collapsed, let the user opt in to full render.
  // `_traceBodyExpanded` is set per-tab when the expand button is clicked.
  const tab = getActiveTab();
  const expanded = tab && tab._traceBodyExpanded === true;
  const shown = expanded ? body : body.slice(0, TRACE_BODY_PREVIEW_CHARS) + '\n…';
  const sizeKb = (body.length / 1024).toFixed(1);
  const label = expanded
    ? `collapse (showing full ${sizeKb} KB)`
    : `show full response body (${sizeKb} KB)`;
  return safeHtml`
    <pre class="trace-code">${shown}</pre>
    <button type="button" class="mt-2 text-[10px] font-semibold tracking-wide px-2 py-1 rounded-sm text-ox-electric border border-ox-electric/50 hover:bg-ox-electric/10 hover:border-ox-electric transition-colors" data-action="toggle-trace-body">${label}</button>`;
}

function renderTraceList(tab) {
  const list = document.getElementById('traceList');
  if (!tab || tab.httpTraceEntries.length === 0) {
    list.innerHTML = '<div class="px-4 py-3 text-[11px] text-ox-dim font-mono">No traced requests yet.</div>';
    return;
  }

  const selectedId = ensureTraceSelection(tab);
  const html = [...tab.httpTraceEntries].reverse().map(entry => {
    const active = entry.id === selectedId ? ' active' : '';
    const statusClass = traceStatusClass(entry);
    const statusCls = statusClass ? ` ${statusClass}` : '';
    return safeHtml`
      <div class="trace-row${active}" data-action="select-trace" data-trace-id="${entry.id}">
        <div class="trace-meta">
          <span class="trace-pill">${entry.method}</span>
          <span class="trace-pill${statusCls}">${traceStatusLabel(entry)}</span>
          <span>${entry.duration_ms}ms</span>
        </div>
        <div class="trace-url">${compactTraceUrl(entry.url)}</div>
        <div class="trace-meta">${traceOutcomeLabel(entry)}</div>
      </div>`;
  }).join('');
  list.innerHTML = html;
}

export function renderTraceDetail(tab) {
  const detail = document.getElementById('traceDetail');
  const entry = getSelectedTraceEntry(tab);
  if (!entry) {
    detail.innerHTML = '<div class="px-4 py-4 text-[11px] text-ox-dim font-mono">Select a traced request to inspect it.</div>';
    return;
  }

  const activeSubTab = tab?._traceSubTab === 'request' ? 'request' : 'response';
  const statusClass = traceStatusClass(entry);
  const statusCls = statusClass ? ` ${statusClass}` : '';

  const requestActive = activeSubTab === 'request' ? ' active' : '';
  const responseActive = activeSubTab === 'response' ? ' active' : '';
  const actionButtons = activeSubTab === 'request'
    ? safeHtml`
      <button data-action="copy-trace-curl">copy as curl</button>
      <button data-action="copy-trace-request-body"${raw(entry.request_body_preview ? '' : ' disabled')}>copy body</button>`
    : safeHtml`
      <button data-action="copy-trace-response-body"${raw(entry.response_body_preview ? '' : ' disabled')}>copy body</button>`;

  const sections = [];
  if (activeSubTab === 'request') {
    sections.push(safeHtml`
      <div class="trace-section">
        <div class="trace-section-title">Headers</div>
        ${raw(renderTraceHeaders(entry.request_headers))}
      </div>`);
    sections.push(safeHtml`
      <div class="trace-section">
        <div class="trace-section-title">Body</div>
        ${raw(renderTraceBody(entry.request_body_preview, 'No request body captured.'))}
      </div>`);
  } else {
    sections.push(safeHtml`
      <div class="trace-section">
        <div class="trace-section-title">Headers</div>
        ${raw(renderTraceHeaders(entry.response_headers))}
      </div>`);
    sections.push(safeHtml`
      <div class="trace-section">
        <div class="trace-section-title">Body Preview</div>
        ${raw(renderTraceBody(entry.response_body_preview, 'No response body preview captured.'))}
      </div>`);

    if (entry.redirect_location) {
      sections.push(safeHtml`
        <div class="trace-section">
          <div class="trace-section-title">Redirect</div>
          <div class="trace-code">${entry.redirect_location}</div>
        </div>`);
    }

    if (entry.error) {
      sections.push(safeHtml`
        <div class="trace-section">
          <div class="trace-section-title">Error</div>
          <pre class="trace-code">${entry.error}</pre>
        </div>`);
    }
  }

  const html = safeHtml`
    <div class="trace-section">
      <div class="flex items-center gap-2 mb-2">
        <span class="trace-pill">${entry.method}</span>
        <span class="trace-pill${statusCls}">${traceStatusLabel(entry)}</span>
        <span class="trace-meta">${entry.duration_ms}ms</span>
      </div>
      <div class="trace-url">${entry.url}</div>
    </div>
    <div class="trace-subtabs">
      <div class="trace-subtab${requestActive}" data-action="select-trace-subtab" data-subtab="request">Request</div>
      <div class="trace-subtab${responseActive}" data-action="select-trace-subtab" data-subtab="response">Response</div>
      <div class="trace-subtab-actions">${raw(actionButtons)}</div>
    </div>
    ${raw(sections.join(''))}`;

  detail.innerHTML = html;
}

export function renderTraceInspector(tab = getActiveTab()) {
  renderTraceSummary(tab);
  renderTraceList(tab);
  renderTraceDetail(tab);
}

function showTraceInspector() {
  const tab = getActiveTab();
  if (!tab) return;
  tab._traceVisible = true;
  renderTraceInspector(tab);
  document.getElementById('traceInspectorPanel').classList.remove('hidden');
  updateTraceToggleState(true);
}

export function hideTraceInspector() {
  const tab = getActiveTab();
  if (tab) tab._traceVisible = false;
  document.getElementById('traceInspectorPanel').classList.add('hidden');
  updateTraceToggleState(false);
}

export function updateTraceToggleState(open) {
  const btn = document.getElementById('btnTraceToggle');
  const chevron = document.getElementById('traceToggleChevron');
  if (!btn || !chevron) return;
  chevron.innerHTML = open ? '&#x25BE;' : '&#x25B4;';
  // Off = dim green (primed / telemetry standby). On = full green
  // with a subtle glow. Distinct from SAP View's amber so the two
  // status-bar toggles read as different kinds of switch at a glance.
  if (open) {
    btn.classList.add('text-ox-green', 'border-ox-green', 'bg-ox-greenGlow');
    btn.classList.remove('text-ox-greenDim', 'border-ox-greenDim/60');
  } else {
    btn.classList.add('text-ox-greenDim', 'border-ox-greenDim/60');
    btn.classList.remove('text-ox-green', 'border-ox-green', 'bg-ox-greenGlow');
  }
}

export function toggleTraceInspector() {
  const panel = document.getElementById('traceInspectorPanel');
  if (panel.classList.contains('hidden')) {
    showTraceInspector();
  } else {
    hideTraceInspector();
  }
}

// POSIX-shell single-quote escape. The resulting curl command runs in bash /
// zsh / git-bash, but cmd.exe and PowerShell use different quoting rules —
// paste into those shells and the quotes will leak through literally.
function shellQuoteForCurl(value) {
  return "'" + String(value).replace(/'/g, `'\"'\"'`) + "'";
}

function traceToCurl(entry) {
  const parts = [
    `curl -X ${shellQuoteForCurl(entry.method)}`,
    `--url ${shellQuoteForCurl(entry.url)}`,
  ];
  for (const header of entry.request_headers || []) {
    parts.push(`-H ${shellQuoteForCurl(`${header.name}: ${header.value}`)}`);
  }
  if (entry.request_body_preview) {
    parts.push(`--data-raw ${shellQuoteForCurl(entry.request_body_preview)}`);
  }
  return parts.join(' ');
}

export async function copySelectedTraceAsCurl() {
  const entry = getSelectedTraceEntry(getActiveTab());
  if (!entry) {
    setStatus('No trace selected');
    return;
  }
  await copyToClipboard(traceToCurl(entry), 'curl command');
}

export async function copySelectedTraceRequestBody() {
  const entry = getSelectedTraceEntry(getActiveTab());
  if (!entry || !entry.request_body_preview) {
    setStatus('No request body to copy');
    return;
  }
  await copyToClipboard(entry.request_body_preview, 'request body');
}

export async function copySelectedTraceResponseBody() {
  const entry = getSelectedTraceEntry(getActiveTab());
  if (!entry || !entry.response_body_preview) {
    setStatus('No response body to copy');
    return;
  }
  await copyToClipboard(entry.response_body_preview, 'response body');
}
