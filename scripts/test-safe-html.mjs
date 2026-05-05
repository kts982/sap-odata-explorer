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
//   1. escapeHtml renders HTML-shaped strings as inert text in BOTH
//      element-content and quoted-attribute contexts. The five-char
//      OWASP set (`& < > " '`) is escaped — quote escaping is what
//      closes the attribute-breakout class (see assertion below).
//   2. safeHtml`...${malicious}...` interpolates only the escaped form
//      in both element content and attribute values.
//   3. raw(htmlString) — when called from trusted code holding the
//      closure-private Symbol — passes through unescaped.
//   4. A JSON-shaped { __rawHtml: true, value: '<script>...' } payload
//      CANNOT bypass escaping. JSON has no Symbol type, so the brand
//      check fails and the object is stringified to "[object Object]"
//      instead of being treated as raw HTML.
//
// `escapeHtml` is pure JS now (no `document` dependency) so the script
// runs under vanilla Node with no polyfill or external deps.
//
// Run locally:
//   node scripts/test-safe-html.mjs
// Exits 0 on pass, 1 on any assertion failure. Output is a single line
// per assertion plus a final summary.

import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const HTML_MODULE = resolve(HERE, '..', 'tauri-app', 'src', 'html.js');

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

// 1. escapeHtml on a classic element-content XSS payload.
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

// 2. escapeHtml escapes the full OWASP five-char set, including quotes.
//    Quote escaping is what makes the function safe for attribute
//    contexts; without it `escapeHtml` would silently leave breakouts.
{
  const out = escapeHtml(`a & b < c > d " e ' f`);
  const want = `a &amp; b &lt; c &gt; d &quot; e &#39; f`;
  check(
    'escapeHtml escapes & < > " \' (full OWASP set)',
    out === want,
    want,
    out,
  );
}

// 3. escapeHtml coerces null / undefined / non-string to ''. Avoids
//    TypeErrors on direct calls and keeps safeHtml output stable when
//    an upstream field is missing.
{
  check('escapeHtml(null) → empty string', escapeHtml(null) === '', '', escapeHtml(null));
  check(
    'escapeHtml(undefined) → empty string',
    escapeHtml(undefined) === '',
    '',
    escapeHtml(undefined),
  );
  check('escapeHtml(123) → "123"', escapeHtml(123) === '123', '123', escapeHtml(123));
}

// 4. safeHtml interpolates the escaped form in element content.
{
  const hostile = '<svg/onload=alert(1)>';
  const out = safeHtml`<div>${hostile}</div>`;
  check(
    'safeHtml escapes < in element-content interpolation',
    !out.includes('<svg/onload=alert(1)>') && out.includes('&lt;svg/onload=alert(1)&gt;'),
    'no raw <svg…>; contains &lt;svg…&gt;',
    out,
  );
}

// 5. ATTRIBUTE BREAKOUT — load-bearing for the html.js fix. SAP-controlled
//    metadata flows into `data-*` / `title=""` attributes via safeHtml in
//    several renderers (selection-field chip names, describe-row
//    data-field, value-list F4 button data-prop / title). A `"` in the
//    value would close the attribute and inject new ones; CSP-strict
//    blocks inline-handler EXECUTION today, but the breakout itself is
//    still real (style=, id=, markup-shape attacks; defense-in-depth
//    against any future CSP relaxation). The 5-char escape closes it.
{
  const hostile = `" onclick="alert(1)`;
  const out = safeHtml`<button data-name="${hostile}">click</button>`;
  check(
    'safeHtml escapes " in attribute-context interpolation (breakout closed)',
    out === `<button data-name="&quot; onclick=&quot;alert(1)">click</button>`,
    `<button data-name="&quot; onclick=&quot;alert(1)">click</button>`,
    out,
  );
  check(
    'attribute-breakout output does not contain raw onclick="..."',
    !out.includes(`onclick="alert(1)`),
    `no raw onclick="alert(1)"`,
    out,
  );
}

// 6. Apostrophe-shaped breakout (single-quoted attributes are rarer in
//    this codebase but the contract should still hold).
{
  const hostile = `' onmouseover='alert(1)`;
  const out = safeHtml`<button data-name='${hostile}'>click</button>`;
  check(
    'safeHtml escapes \' in single-quoted attribute interpolation',
    out.includes('&#39;') && !out.includes(`onmouseover='alert(1)`),
    'contains &#39; and no raw onmouseover=…',
    out,
  );
}

// 7. safeHtml leaves the static template scaffolding alone.
{
  const out = safeHtml`<span class="x">${'plain'}</span>`;
  check(
    'safeHtml preserves static template HTML verbatim',
    out === '<span class="x">plain</span>',
    '<span class="x">plain</span>',
    out,
  );
}

// 8. raw() called from this script (which has access to the marker via
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

// 9. JSON-shaped __rawHtml forgery is the load-bearing claim. A SAP
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

// 10. null / undefined interpolations don't blow up and don't print "null".
{
  const out = safeHtml`<p>${null}-${undefined}</p>`;
  check(
    'safeHtml renders null/undefined as empty strings',
    out === '<p>-</p>',
    '<p>-</p>',
    out,
  );
}

// 11. Repeat the parser-side fixture's worst payloads against the
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
