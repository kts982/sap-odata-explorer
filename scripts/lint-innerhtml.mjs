#!/usr/bin/env node
// Lint: forbid bare template-literal innerHTML assignments.
//
// Threat model: SAP `$metadata` (entity names, annotation values, error
// messages) is untrusted input. Any HTML built with
// `el.innerHTML = `...${untrusted}...`` without escaping is XSS-shaped.
// The `safeHtml` tagged-template helper in `tauri-app/src/html.js`
// auto-escapes interpolations; this lint enforces that every innerHTML
// assignment goes through it (or a literal string, or a pre-built
// variable that callers built with safeHtml).
//
// Replaces the earlier grep-based check in CI which only matched
// single-line patterns. Reading each file as a single string and
// applying the same regex catches both:
//   el.innerHTML = `bad ${x}`;        // single-line
//   el.innerHTML =                    // multi-line: backtick on next line
//     `<div>${x}</div>`;
//
// Deliberate limitation — the regex does NOT prove variable
// indirection:
//   const html = `<div>${x}</div>`;
//   el.innerHTML = html;
// The literal isn't on the RHS of an innerHTML assignment. The project
// closes that path by convention: renderer builders must assemble
// variables from safeHtml fragments and pass prebuilt fragments via
// raw() only after their interpolations have already been escaped.
//
// Targets are auto-discovered: every .js file under tauri-app/src/
// (excluding vendor/) plus index.html. Adding a new module no longer
// requires updating an allowlist.

import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

const SRC_DIR = 'tauri-app/src';
const EXCLUDE_DIRS = new Set(['vendor', 'fonts']);
const EXTRA_TARGETS = [join(SRC_DIR, 'index.html')];

function discoverJsFiles(dir) {
  const out = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.isDirectory()) {
      if (EXCLUDE_DIRS.has(entry.name)) continue;
      out.push(...discoverJsFiles(join(dir, entry.name)));
    } else if (entry.isFile() && entry.name.endsWith('.js')) {
      out.push(join(dir, entry.name));
    }
  }
  return out;
}

const TARGETS = [...discoverJsFiles(SRC_DIR), ...EXTRA_TARGETS]
  .map(p => p.replace(/\\/g, '/'))
  .sort();

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

console.log(`ok: no bare template-literal innerHTML sites (${TARGETS.length} files scanned)`);
