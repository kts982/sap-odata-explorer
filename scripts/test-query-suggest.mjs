#!/usr/bin/env node
// Query-bar autocomplete token logic — tauri-app/src/suggest.js.
//
// Run locally:
//   node scripts/test-query-suggest.mjs
// Exits 0 on pass, 1 on any assertion failure.

import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const MODULE = resolve(HERE, '..', 'tauri-app', 'src', 'suggest.js');
const { suggestionsFor } = await import(`file://${MODULE.replace(/\\/g, '/')}`);

const info = {
  properties: [
    { name: 'Product', label: 'Product', edm_type: 'Edm.String' },
    { name: 'ProductDescription', label: 'Description', edm_type: 'Edm.String' },
    { name: 'EWMWarehouse', label: 'Warehouse Number', edm_type: 'Edm.String' },
    { name: 'Quantity', label: 'Quantity', edm_type: 'Edm.Decimal' },
    { name: 'SAP__Messages', label: '', edm_type: 'Collection(SAP__self.SAP__Message)' },
  ],
  nav_properties: [{ name: '_SerialNumber' }, { name: '_Batch' }],
};

let failures = 0;
let count = 0;
function check(label, actual, expected) {
  count++;
  const a = JSON.stringify(actual);
  const e = JSON.stringify(expected);
  if (a === e) {
    console.log(`ok    ${label}`);
  } else {
    failures++;
    console.log(`FAIL  ${label}\n      expected: ${e}\n      actual:   ${a}`);
  }
}
const values = (mode, text, caret = text.length, version = 'V4') =>
  suggestionsFor(mode, text, caret, info, version).items.map(i => i.value);
const range = (mode, text, caret = text.length) => {
  const r = suggestionsFor(mode, text, caret, info, 'V4');
  return [r.from, r.to];
};

check('select: prefix match first, system fields hidden', values('select', 'Prod'), ['Product', 'ProductDescription']);
check('select: token after a comma', values('select', 'Product,EWM'), ['EWMWarehouse']);
check('select: replaces only the current token', range('select', 'Product,EWM'), [8, 11]);
check('select: matches labels too', values('select', 'number'), ['EWMWarehouse']);
check('expand: navigation properties', values('expand', '_'), ['_SerialNumber', '_Batch']);
check('orderby: direction after a property', values('orderby', 'Product '), ['asc', 'desc']);
check('orderby: property in the next clause', values('orderby', 'Product desc,Qu'), ['Quantity']);
check('filter: properties and functions', values('filter', 'sta'), ['startswith(']);
check('filter: operators after a property', values('filter', 'Product '), ['eq', 'ne', 'gt', 'ge', 'lt', 'le']);
check('filter: operator prefix', values('filter', 'Product g'), ['gt', 'ge']);
check('filter: logical after a string literal', values('filter', "Product eq 'A' "), ['and', 'or']);
check('filter: logical after a number', values('filter', 'Quantity gt 5 '), ['and', 'or']);
check('filter: nothing inside a string literal', values('filter', "Product eq 'Pro"), []);
check('filter: V4 contains, V2 substringof', [values('filter', 'cont', undefined, 'V4'), values('filter', 'subst', undefined, 'V2')], [['contains('], ['substringof(']]);
check('filter: no popup on a bare space', values('filter', ''), []);
check('no describe info → nothing', suggestionsFor('select', 'P', 1, null, 'V4').items, []);
const op = suggestionsFor('filter', 'Product ', 8, info, 'V4').items[0];
check('operator insert adds a trailing space', op.insert, 'eq ');
const top = suggestionsFor('filter', 'EWMWare', 7, info, 'V4').items[0];
check('top-level property insert chains to operators', top.insert, 'EWMWarehouse ');
const inFn = suggestionsFor('filter', 'startswith(EWMWare', 18, info, 'V4').items[0];
check('property inside a function call has no trailing space', inFn.insert, 'EWMWarehouse');

console.log(failures ? `\n${failures} failure(s)` : `\nall ${count} assertions passed`);
process.exit(failures ? 1 : 0);
