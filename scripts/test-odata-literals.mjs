#!/usr/bin/env node
// OData filter-literal formatting — tauri-app/src/format.js.
//
// The desktop app writes `$filter` clauses for the user from four places
// (filter bar, Fiori SelectionVariant button, value-help picker, result
// cell click). All of them go through `formatODataLiteral(value,
// edmType, version)`. V2 and V4 spell most non-string literals
// differently, and values arrive in the shapes the UI sees: what the user
// typed, or a raw V2/V4 JSON value (`/Date(…)/`, `PT…`, ISO).
//
// Run locally:
//   node scripts/test-odata-literals.mjs
// Exits 0 on pass, 1 on any assertion failure.

import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const FORMAT_MODULE = resolve(HERE, '..', 'tauri-app', 'src', 'format.js');

const { formatODataLiteral, containsClause, serviceODataVersion } = await import(
  `file://${FORMAT_MODULE.replace(/\\/g, '/')}`
);

let failures = 0;
function check(label, actual, expected) {
  if (actual === expected) {
    console.log(`ok    ${label}`);
  } else {
    failures++;
    console.log(`FAIL  ${label}\n      expected: ${expected}\n      actual:   ${actual}`);
  }
}

// [value, edmType, version, expected]
const cases = [
  // Strings: quoted, apostrophes doubled, whitespace preserved.
  ["O'Brien", 'Edm.String', 'V4', "'O''Brien'"],
  [' padded ', 'Edm.String', 'V2', "' padded '"],
  ['01', 'Edm.String', 'V2', "'01'"],
  // Numbers and booleans: bare; non-numeric input stays quoted.
  ['5', 'Edm.Int32', 'V4', '5'],
  ['12.50', 'Edm.Decimal', 'V2', '12.50'],
  ['abc', 'Edm.Int32', 'V4', "'abc'"],
  ['true', 'Edm.Boolean', 'V2', 'true'],
  // Guid: V2 prefixed, V4 bare (also unwraps an already-prefixed value).
  ['005056a2-1234-1edb-8000-000000000001', 'Edm.Guid', 'V2', "guid'005056a2-1234-1edb-8000-000000000001'"],
  ['005056a2-1234-1edb-8000-000000000001', 'Edm.Guid', 'V4', '005056a2-1234-1edb-8000-000000000001'],
  ["guid'005056a2-1234-1edb-8000-000000000001'", 'Edm.Guid', 'V4', '005056a2-1234-1edb-8000-000000000001'],
  // V2 DateTime: offset-less ISO in datetime'…'.
  ['/Date(1700000000000)/', 'Edm.DateTime', 'V2', "datetime'2023-11-14T22:13:20'"],
  ['2026-01-01', 'Edm.DateTime', 'V2', "datetime'2026-01-01T00:00:00'"],
  ['2026-01-01T10:30:00', 'Edm.DateTime', 'V2', "datetime'2026-01-01T10:30:00'"],
  // DateTimeOffset: V2 prefixed, V4 bare.
  ['/Date(1700000000000+0000)/', 'Edm.DateTimeOffset', 'V2', "datetimeoffset'2023-11-14T22:13:20Z'"],
  ['2026-01-01T10:30:00Z', 'Edm.DateTimeOffset', 'V4', '2026-01-01T10:30:00Z'],
  ['2026-01-01', 'Edm.DateTimeOffset', 'V4', '2026-01-01T00:00:00Z'],
  // V4 Date / TimeOfDay: bare.
  ['2026-01-01', 'Edm.Date', 'V4', '2026-01-01'],
  ['/Date(1700000000000)/', 'Edm.Date', 'V4', '2023-11-14'],
  ['13:20:05', 'Edm.TimeOfDay', 'V4', '13:20:05'],
  ['PT13H20M05S', 'Edm.TimeOfDay', 'V4', '13:20:05'],
  // V2 Time: time'PT…'.
  ['PT13H20M05S', 'Edm.Time', 'V2', "time'PT13H20M05S'"],
  ['13:20', 'Edm.Time', 'V2', "time'PT13H20M00S'"],
  // Unknown type → quoted string; null → null.
  ['x', 'Custom.Type', 'V4', "'x'"],
  [null, 'Edm.String', 'V4', 'null'],
];

for (const [value, type, version, expected] of cases) {
  check(`${version} ${type} ${JSON.stringify(value)}`, formatODataLiteral(value, type, version), expected);
}

check('V4 contains', containsClause('Name', "'ab'", 'V4'), "contains(Name,'ab')");
check('V2 contains → substringof', containsClause('Name', "'ab'", 'V2'), "substringof('ab',Name)");

check('version from metadata wins', serviceODataVersion({ serviceVersion: 'V2', servicePath: '/sap/opu/odata4/x' }), 'V2');
check('version from V2 path', serviceODataVersion({ servicePath: '/sap/opu/odata/sap/ZSRV' }), 'V2');
check('version from V4 path', serviceODataVersion({ servicePath: '/sap/opu/odata4/sap/zsrv/srvd/sap/zsrv/0001' }), 'V4');

console.log(failures ? `\n${failures} failure(s)` : `\nall ${cases.length + 5} assertions passed`);
process.exit(failures ? 1 : 0);
