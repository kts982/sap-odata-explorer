#!/usr/bin/env node
// Renderer-side malicious-content test — pairs with
// crates/core/tests/malicious_content.rs (the parser-side proof).
//
// The escape boundary is:
//
//   parser keeps data intact  ─→  renderer escapes
//
// This script imports the real `escapeHtml` / `safeHtml` / `raw` from
// tauri-app/src/html.js and asserts:
//
//   1. escapeHtml renders HTML-shaped strings as inert text (`<` → `&lt;`,
//      `&` → `&amp;`, `>` → `&gt;`).
//   2. safeHtml`...${malicious}...` interpolates only the escaped form.
//   3. raw(htmlString) — when called from trusted code holding the
//      closure-private Symbol — passes through unescaped.
//   4. A JSON-shaped { __rawHtml: true, value: '<script>...' } payload
//      CANNOT bypass escaping. JSON has no Symbol type, so the brand
//      check fails and the object is stringified to "[object Object]"
//      instead of being treated as raw HTML.
//
// `escapeHtml` uses `document.createElement('div')` + `textContent`
// in production. We polyfill the minimum surface here — a div that
// HTML-escapes its `textContent` setter — so the script runs under
// vanilla Node with no dependencies. The polyfill mirrors what every
// browser does internally for textContent → innerHTML readback (escape
// `&`, `<`, `>` — quotes are not touched, matching real DOM behaviour).
//
// Run locally:
//   node scripts/test-safe-html.mjs
// Exits 0 on pass, 1 on any assertion failure. Output is a single line
// per assertion plus a final summary.

import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const HTML_MODULE = resolve(HERE, '..', 'tauri-app', 'src', 'html.js');

// ── Minimal DOM polyfill ──
// Real DOM rule: assigning a string to `textContent` and reading
// `innerHTML` back returns the string with `&`, `<`, `>` replaced by
// their HTML entities. Quotes are left alone — they're only dangerous
// in attribute contexts, which the project does not interpolate into.
function htmlEscapeReference(str) {
  return String(str)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
}

globalThis.document = {
  createElement(_tag) {
    let stored = '';
    return {
      set textContent(value) {
        stored = htmlEscapeReference(value);
      },
      get innerHTML() {
        return stored;
      },
    };
  },
};

const { escapeHtml, safeHtml, raw } = await import(`file://${HTML_MODULE.replace(/\\/g, '/')}`);

let failures = 0;
function check(label, condition, expected, got) {
  if (condition) {
    console.log(`ok   ${label}`);
  } else {
    console.error(`FAIL ${label}`);
    if (expected !== undefined) console.error(`     expected: ${expected}`);
    if (got !== undefined) console.error(`     got:      ${got}`);
    failures += 1;
  }
}

// 1. escapeHtml on a classic XSS payload.
{
  const payload = '<img src=x onerror=alert(1)>';
  const out = escapeHtml(payload);
  check(
    'escapeHtml neutralises <img onerror>',
    out === '&lt;img src=x onerror=alert(1)&gt;',
    '&lt;img src=x onerror=alert(1)&gt;',
    out,
  );
}

// 2. escapeHtml on entities that need escaping in element content.
{
  const out = escapeHtml('a & b < c > d');
  check(
    'escapeHtml escapes & < > in element content',
    out === 'a &amp; b &lt; c &gt; d',
    'a &amp; b &lt; c &gt; d',
    out,
  );
}

// 3. safeHtml interpolates the escaped form, not the raw payload.
{
  const hostile = '<svg/onload=alert(1)>';
  const out = safeHtml`<div data-name="${hostile}">${hostile}</div>`;
  // The element-content interpolation must be escaped. The attribute
  // interpolation should also not contain an unescaped `<` that could
  // break out of the attribute. (Attribute escaping is not a goal of
  // textContent-based escapeHtml, but `<` and `>` are still neutralised.)
  check(
    'safeHtml escapes < in element-content interpolation',
    !out.includes('<svg/onload=alert(1)>') &&
      out.includes('&lt;svg/onload=alert(1)&gt;'),
    'no raw <svg…>; contains &lt;svg…&gt;',
    out,
  );
}

// 4. safeHtml leaves the static template scaffolding alone.
{
  const out = safeHtml`<span class="x">${'plain'}</span>`;
  check(
    'safeHtml preserves static template HTML verbatim',
    out === '<span class="x">plain</span>',
    '<span class="x">plain</span>',
    out,
  );
}

// 5. raw() called from this script (which has access to the marker via
// the html.js closure) passes through unescaped — the trusted-callsite
// path the project uses for already-safeHtml'd fragments.
{
  const fragment = '<b>already escaped</b>';
  const out = safeHtml`<p>${raw(fragment)}</p>`;
  check(
    'raw() pass-through preserves trusted HTML',
    out === '<p><b>already escaped</b></p>',
    '<p><b>already escaped</b></p>',
    out,
  );
}

// 6. JSON-shaped __rawHtml forgery is the load-bearing claim. A SAP
// payload like { "__rawHtml": true, "value": "<script>..." } must NOT
// be treated as raw — JSON cannot construct Symbol-keyed properties,
// so the brand check on RAW_HTML_MARKER fails and safeHtml falls back
// to escapeHtml(String(obj)), which yields "[object Object]".
{
  const forged = JSON.parse('{"__rawHtml": true, "value": "<script>alert(1)</script>"}');
  const out = safeHtml`<div>${forged}</div>`;
  check(
    'JSON-shaped __rawHtml cannot forge raw() — Symbol brand is closure-private',
    !out.includes('<script>') && out.includes('[object Object]'),
    'no raw <script>; contains [object Object]',
    out,
  );
}

// 7. null / undefined interpolations don't blow up and don't print "null".
{
  const out = safeHtml`<p>${null}-${undefined}</p>`;
  check(
    'safeHtml renders null/undefined as empty strings',
    out === '<p>-</p>',
    '<p>-</p>',
    out,
  );
}

// 8. Repeat the parser-side fixture's worst payloads against the
// renderer to close the loop end-to-end.
{
  const cases = [
    'Order<img>Type',
    '<img src=x onerror=alert(1)>',
    'javascript:alert("hi")',
    '<svg/onload=alert(1)>',
    'evil<script>',
  ];
  for (const c of cases) {
    const out = safeHtml`<span>${c}</span>`;
    check(
      `parser-fixture payload escaped: ${JSON.stringify(c).slice(0, 40)}`,
      !out.includes('<img>') &&
        !out.includes('<script>') &&
        !out.includes('<svg/onload') &&
        !out.includes('<img src=x'),
      'no raw HTML tags',
      out,
    );
  }
}

if (failures > 0) {
  console.error(`\n${failures} renderer-escape assertion(s) failed.`);
  process.exit(1);
}
console.log('\nok: renderer escaping contract holds for all malicious-content fixtures.');
