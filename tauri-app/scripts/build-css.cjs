const { spawnSync } = require('node:child_process');
const path = require('node:path');

// Tailwind v4's @tailwindcss/cli package only exposes itself via `bin`; its
// JS entrypoint isn't in "exports". Resolve the package.json and point at the
// declared bin script directly.
const cwd = path.resolve(__dirname, '..');
const pkgJson = require.resolve('@tailwindcss/cli/package.json', { paths: [cwd] });
const pkg = require(pkgJson);
const cliPath = path.resolve(path.dirname(pkgJson), pkg.bin.tailwindcss);

const result = spawnSync(
  process.execPath,
  [cliPath, '-i', 'src/input.css', '-o', 'src/style.css', '--minify'],
  {
    cwd,
    stdio: 'inherit',
  }
);

if (result.error) {
  throw result.error;
}

process.exit(result.status ?? 1);
