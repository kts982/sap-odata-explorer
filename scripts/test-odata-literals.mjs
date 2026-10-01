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

const { formatODataLiteral, containsClause, serviceODataVersion, shellArg, buildCliCommand } = await import(
  `file://${FORMAT_MODULE.replace(/\\/g, '/')}`
);

let failures = 0;
function check(label, actual, expected) {
  if (JSON.stringify(actual) === JSON.stringify(expected)) {
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

// "Copy as CLI": quoting that pastes into Bash and PowerShell alike.
check('plain arg stays bare', shellArg('A_SalesOrder'), 'A_SalesOrder');
check('filter with quotes → double quotes', shellArg("Plant eq '1000'"), `"Plant eq '1000'"`);
check('$ forces single quotes', shellArg('a$b'), `'a$b'`);
check('single quote inside single quotes', shellArg("a$'b"), `'a$'\\''b'`);
check(
  'full command',
  buildCliCommand('DEV', '/sap/opu/odata4/sap/zsrv/srvd/sap/zsrv/0001', {
    entity_set: 'Orders',
    select: 'ID,Status',
    filter: "Status eq 'OPEN'",
    expand: null,
    orderby: '',
    top: 5,
    skip: null,
  }),
  `sap-odata -p DEV -s /sap/opu/odata4/sap/zsrv/srvd/sap/zsrv/0001 run Orders --select ID,Status --filter "Status eq 'OPEN'" --top 5`,
);

// Paging + export helpers (stats bar).
const { extractTotalCount, nextSkiptoken, pagingInfo, toDelimited } = await import(
  `file://${FORMAT_MODULE.replace(/\\/g, '/')}`
);
const v4Page = {
  '@odata.count': 253,
  '@odata.nextLink': '/sap/opu/odata4/x/CountryVH?sap-client=100&$count=true&$skiptoken=100',
  value: [],
};
check('V4 total', extractTotalCount(v4Page), 253);
check('V2 total (string __count)', extractTotalCount({ d: { __count: '11916', results: [] } }), 11916);
check('no total', extractTotalCount({ value: [] }), null);
check('V4 next-link skiptoken', nextSkiptoken(v4Page), '100');
check('V2 __next skiptoken', nextSkiptoken({ d: { __next: "https://h/sap/opu/odata/sap/S/Set?$skiptoken='A%2C1'" } }), "'A,1'");
check('no next link', nextSkiptoken({ value: [] }), null);
const p1 = pagingInfo({ '@odata.count': 45, value: [] }, { top: 20, skip: null }, 20);
check('page 1 of 45: next skips 20, no prev', [p1.nextSkip, p1.prevSkip, p1.hasMore], [20, null, true]);
const p3 = pagingInfo({ '@odata.count': 45, value: [] }, { top: 20, skip: 40 }, 5);
check('last page: no next, prev 20', [p3.nextSkip, p3.prevSkip, p3.hasMore], [null, 20, false]);
const pNoCount = pagingInfo({ value: [] }, { top: 20, skip: null }, 20);
check('full page without count: assume more', pNoCount.hasMore, true);
const pToken = pagingInfo(v4Page, { top: null, skip: null, skiptoken: '100' }, 100);
check('server-driven: next by skiptoken, offset from numeric token', [pToken.nextSkiptoken, pToken.offset, pToken.prevSkip], ['100', 100, 0]);
check(
  'TSV: header, nulls, nested JSON, quoting',
  toDelimited(
    [
      { __metadata: { uri: 'x' }, ID: '1', Name: 'Tab\there', Note: null, Items: [{ a: 1 }] },
      { __metadata: { uri: 'y' }, ID: '2', Name: 'Say "hi"', Note: 'line1\nline2', Items: [] },
    ],
    '\t',
  ),
  // Nested JSON contains quotes, so it is quoted too (Excel unquotes it).
  'ID\tName\tNote\tItems\r\n1\t"Tab\there"\t\t"[{""a"":1}]"\r\n2\t"Say ""hi"""\t"line1\nline2"\t[]',
);
check('CSV quotes commas', toDelimited([{ A: 'x,y', B: 3 }], ','), 'A,B\r\n"x,y",3');

// Row drill-down key predicate.
const { keyPredicate } = await import(`file://${FORMAT_MODULE.replace(/\\/g, '/')}`);
const guidRow = {
  ParentHandlingUnitUUID: '00000000-0000-0000-0000-000000000000',
  StockItemUUID: '6a1f0c2e-1b2d-4c3e-8f40-0123456789ab',
  Product: "O'Brien",
};
const types = { ParentHandlingUnitUUID: 'Edm.Guid', StockItemUUID: 'Edm.Guid', Product: 'Edm.String' };
check(
  'V4 composite GUID key',
  keyPredicate(['ParentHandlingUnitUUID', 'StockItemUUID'], guidRow, types, 'V4'),
  'ParentHandlingUnitUUID=00000000-0000-0000-0000-000000000000,StockItemUUID=6a1f0c2e-1b2d-4c3e-8f40-0123456789ab',
);
check('V2 GUID key prefixed', keyPredicate(['StockItemUUID'], guidRow, types, 'V2'), "StockItemUUID=guid'6a1f0c2e-1b2d-4c3e-8f40-0123456789ab'");
check('string key quoted and escaped', keyPredicate(['Product'], guidRow, types, 'V4'), "Product='O''Brien'");
check('missing key value → null', keyPredicate(['Missing'], guidRow, types, 'V4'), null);

// "copy for AI" markdown.
const { describeAsMarkdown } = await import(`file://${FORMAT_MODULE.replace(/\\/g, '/')}`);
const md = describeAsMarkdown(
  {
    name: 'OrderType',
    keys: ['ID'],
    header_info: { type_name: 'Order', type_name_plural: 'Orders', title_path: 'ID' },
    properties: [
      { name: 'ID', edm_type: 'Edm.String', max_length: 10, label: 'Order', is_key: true },
      { name: 'Plant', edm_type: 'Edm.String', label: 'Plant | Site', required_in_filter: true, text_path: 'PlantName', value_list_references: ['../f4'] },
      { name: 'SAP__Messages', edm_type: 'Collection(SAP__self.SAP__Message)' },
    ],
    nav_properties: [{ name: '_Items', target_type: 'n.ItemType', multiplicity: '*' }],
    selection_fields: ['Plant'],
    line_item: [{ value_path: 'ID' }, { value_path: 'Plant' }],
    sort_order: [{ property: 'ID', descending: true }],
    countable: false,
    fiori_readiness: [
      { severity: 'pass', code: 'profile', message: 'Evaluated as list_report.' },
      { severity: 'warn', code: 'text_missing', message: 'Code-looking properties without Common.Text: X' },
    ],
  },
  { entitySet: 'Orders', servicePath: '/sap/opu/odata4/x/0001', version: 'V4' },
);
check('markdown: heading + context', md.split('\n').slice(0, 2), ['## Orders (OrderType)', '/sap/opu/odata4/x/0001 · OData V4']);
check('markdown: key row', md.includes('| ID | String(10) | Order | key |'), true);
check('markdown: notes + escaped pipe', md.includes('| Plant | String | Plant \\| Site | required in filter, text: PlantName, value help |'), true);
check('markdown: system fields skipped', md.includes('SAP__Messages'), false);
check('markdown: navigation + annotations', [md.includes('_Items → ItemType (*)'), md.includes('Default sort: ID desc'), md.includes('Capabilities: no $count')], [true, true, true]);
check('markdown: lint findings without the profile banner', [md.includes('- warn text_missing'), md.includes('profile:')], [true, false]);

console.log(failures ? `\n${failures} failure(s)` : `\nall ${cases.length + 32} assertions passed`);
process.exit(failures ? 1 : 0);
