const { spawnSync } = require('node:child_process');
const path = require('node:path');

// Tailwind 3 bundles Browserslist/caniuse-lite inside its peer bundle.
// That stale-data warning cannot be refreshed from this app's lockfile, so
// suppress it here until the frontend moves to Tailwind 4.
const env = {
  ...process.env,
  BROWSERSLIST_IGNORE_OLD_DATA: '1',
};

const cwd = path.resolve(__dirname, '..');
const cliPath = require.resolve('tailwindcss/lib/cli.js', { paths: [cwd] });
const result = spawnSync(
  process.execPath,
  [cliPath, '-i', 'src/input.css', '-o', 'src/style.css', '--minify'],
  {
    cwd,
    env,
    stdio: 'inherit',
  }
);

if (result.error) {
  throw result.error;
}

process.exit(result.status ?? 1);
