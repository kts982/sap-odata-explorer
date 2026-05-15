#!/usr/bin/env node
// Stage release artifacts under dist/v<tag>/ with the canonical filenames
// referenced by the README + docs/RELEASE-CHECKLIST.md.
//
// The portable GUI exe is the reason this script exists: Tauri writes it as
// `target/release/sap-odata-explorer-app.exe`, but the release page expects
// `SAP-OData-Explorer_<ver>_portable.exe` so the asset table in the README
// stays accurate. Renaming by hand has been forgotten once already (alpha.2
// shipped without it). The script also regenerates SHA256SUMS.txt over the
// final filenames so the hashes line up with what's actually uploaded.
//
// Run after `cargo tauri build` + `cargo build --release -p sap-odata-cli`
// from the repo root:
//
//   node scripts/stage-release-assets.mjs            # version from tauri.conf.json
//   node scripts/stage-release-assets.mjs --tag v0.1.0-alpha.3
//
// The `tag` is what the GitHub release will be named (used for the dist/v...
// directory). The artifact version embedded in filenames comes from
// tauri.conf.json regardless of the tag, because Tauri bakes it in at build
// time.

import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, copyFileSync, writeFileSync } from 'node:fs';
import { join, basename } from 'node:path';
import { argv, exit } from 'node:process';

const REPO_ROOT = new URL('..', import.meta.url).pathname.replace(/^\//, '');

function arg(name) {
  const i = argv.indexOf(name);
  return i >= 0 ? argv[i + 1] : null;
}

function readTauriVersion() {
  const conf = JSON.parse(
    readFileSync(join(REPO_ROOT, 'tauri-app', 'src-tauri', 'tauri.conf.json'), 'utf8'),
  );
  if (!conf.version) throw new Error('no version field in tauri.conf.json');
  return conf.version;
}

function sha256(path) {
  const buf = readFileSync(path);
  return createHash('sha256').update(buf).digest('hex');
}

const version = readTauriVersion();
const tag = arg('--tag') ?? `v${version}`;
const distDir = join(REPO_ROOT, 'dist', tag);

// Source → destination mapping. Tauri's build emits the two installers and
// the GUI exe with the productName `SAP OData Explorer` (spaces) baked in,
// since that's the user-facing name in tauri.conf.json. The release page
// canonicalises filenames to the dashed form `SAP-OData-Explorer_<ver>_...`
// so they sort cleanly and don't carry URL-encoded `%20`s — that rename
// happens here, including the portable rename (sap-odata-explorer-app.exe ->
// SAP-OData-Explorer_<ver>_portable.exe).
const assets = [
  {
    src: join(REPO_ROOT, 'target', 'release', 'bundle', 'msi', `SAP OData Explorer_${version}_x64_en-US.msi`),
    dest: `SAP-OData-Explorer_${version}_x64_en-US.msi`,
  },
  {
    src: join(REPO_ROOT, 'target', 'release', 'bundle', 'nsis', `SAP OData Explorer_${version}_x64-setup.exe`),
    dest: `SAP-OData-Explorer_${version}_x64-setup.exe`,
  },
  {
    src: join(REPO_ROOT, 'target', 'release', 'sap-odata-explorer-app.exe'),
    dest: `SAP-OData-Explorer_${version}_portable.exe`,
  },
  {
    src: join(REPO_ROOT, 'target', 'release', 'sap-odata.exe'),
    dest: 'sap-odata.exe',
  },
];

mkdirSync(distDir, { recursive: true });

const missing = assets.filter(a => !existsSync(a.src));
if (missing.length > 0) {
  console.error('Missing build artifacts — run cargo tauri build + cargo build --release first:');
  for (const m of missing) console.error(`  - ${m.src}`);
  exit(1);
}

const sums = [];
for (const a of assets) {
  const destPath = join(distDir, a.dest);
  copyFileSync(a.src, destPath);
  const hash = sha256(destPath);
  sums.push(`${hash}  ${a.dest}`);
  console.log(`staged: ${a.dest}  (sha256: ${hash.slice(0, 12)}...)`);
}

const sumsPath = join(distDir, 'SHA256SUMS.txt');
writeFileSync(sumsPath, sums.join('\n') + '\n');
console.log(`staged: SHA256SUMS.txt`);
console.log(`\nDone. Assets in ${distDir}`);
console.log(`Upload these 5 files to the GitHub release for tag ${tag}.`);
