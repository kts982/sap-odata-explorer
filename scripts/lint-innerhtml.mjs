#!/usr/bin/env node
// Lint: forbid bare template-literal innerHTML assignments.
//
// Threat model: SAP `$metadata` (entity names, annotation values, error
// messages) is untrusted input. Any HTML built with
// `el.innerHTML = `...${untrusted}...`` without escaping is XSS-shaped.
// The `safeHtml` tagged-template helper in `tauri-app/src/app.js`
// auto-escapes interpolations; this lint enforces that every innerHTML
// assignment goes through it (or a literal string, or a pre-built
// variable that callers built with safeHtml).
//
// Replaces the earlier grep-based check in CI which only matched
// single-line patterns. Reading the file as a single string and
// applying the same regex catches both:
//   el.innerHTML = `bad ${x}`;        // single-line
//   el.innerHTML =                    // multi-line: backtick on next line
//     `<div>${x}</div>`;
//
// Known limitation — the regex still does NOT catch variable
// indirection:
//   const html = `<div>${x}</div>`;
//   el.innerHTML = html;
// The literal isn't on the RHS of an innerHTML assignment, so a
// surface-level regex misses it. Closing that gap requires either AST
// flow analysis (overkill for one project file) or — more practically —
// converting the remaining BUILDER functions in app.js so every
// interpolation passes through safeHtml internally. Once that's done
// the variable RHS is *syntactically* untrusted but the data flow is
// closed by construction. See project_security_hardening.md, item 1.

import { readFileSync } from 'node:fs';

const TARGETS = [
  'tauri-app/src/api.js',
  'tauri-app/src/app.js',
  'tauri-app/src/auth.js',
  'tauri-app/src/favorites.js',
  'tauri-app/src/format.js',
  'tauri-app/src/html.js',
  'tauri-app/src/index.html',
  'tauri-app/src/services.js',
  'tauri-app/src/state.js',
  'tauri-app/src/status.js',
  'tauri-app/src/tabs.js',
];

const RE = /\.innerHTML\s*=\s*`/g;

let violations = 0;
for (const file of TARGETS) {
  let code;
  try {
    code = readFileSync(file, 'utf8');
  } catch (err) {
    console.error(`::error::cannot read ${file}: ${err.message}`);
    process.exit(2);
  }

  for (const m of code.matchAll(RE)) {
    const before = code.slice(0, m.index);
    const line = before.split('\n').length;
    // Compact preview that flattens newlines so the annotation sits on one line.
    const snippet = code
      .slice(m.index, m.index + 80)
      .replace(/\s+/g, ' ')
      .trim();
    console.error(
      `::error file=${file},line=${line}::bare template-literal innerHTML — wrap with safeHtml\`...\`. Got: ${snippet}…`,
    );
    violations += 1;
  }
}

if (violations > 0) {
  console.error(
    `\n${violations} violation(s). See CONTRIBUTING.md "Always escape innerHTML interpolations".`,
  );
  process.exit(1);
}

console.log('ok: no bare template-literal innerHTML sites');
