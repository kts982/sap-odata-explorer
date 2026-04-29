// Copies the small subset of @tauri-apps/api we use (core.js + tslib) from
// node_modules into src/vendor/. The browser cannot resolve the bare specifier
// `@tauri-apps/api/core` on its own; an import map in index.html maps that
// specifier to `./vendor/tauri-core.js`, and `tauri-core.js` keeps its original
// relative import `./external/tslib/tslib.es6.js` working by mirroring the
// directory layout. Re-run this script after `npm install` whenever
// `@tauri-apps/api` is bumped.

const fs = require('node:fs');
const path = require('node:path');

const tauriAppDir = path.resolve(__dirname, '..');
const apiPkg = path.dirname(
  require.resolve('@tauri-apps/api/package.json', { paths: [tauriAppDir] }),
);
const vendorDir = path.join(tauriAppDir, 'src', 'vendor');

const files = [
  { src: 'core.js', dst: 'tauri-core.js' },
  { src: 'external/tslib/tslib.es6.js', dst: 'external/tslib/tslib.es6.js' },
];

for (const { src, dst } of files) {
  const from = path.join(apiPkg, src);
  const to = path.join(vendorDir, dst);
  fs.mkdirSync(path.dirname(to), { recursive: true });
  fs.copyFileSync(from, to);
  console.log(`vendored ${src} -> src/vendor/${dst}`);
}
