// ── Query-bar suggestions (pure) ──
//
// No DOM, no imports — the logic behind autocomplete.js, kept separate so
// scripts/test-query-suggest.mjs can run it under plain Node.

const MAX_SUGGESTIONS = 12;
const COMPARISON = ['eq', 'ne', 'gt', 'ge', 'lt', 'le'];
const LOGICAL = ['and', 'or'];
const FUNCTIONS_COMMON = ['startswith(', 'endswith(', 'tolower(', 'toupper(', 'length(', 'year(', 'month(', 'day(', 'not '];

function isSystemName(name) {
  return name.startsWith('SAP__') || name.startsWith('__');
}

function rank(candidates, prefix) {
  const p = prefix.toLowerCase();
  const starts = [];
  const contains = [];
  for (const c of candidates) {
    const hay = c.value.toLowerCase();
    const label = (c.detail || '').toLowerCase();
    if (hay.startsWith(p)) starts.push(c);
    else if (p && (hay.includes(p) || label.includes(p))) contains.push(c);
  }
  return [...starts, ...contains].slice(0, MAX_SUGGESTIONS);
}

function propertyCandidates(info) {
  return (info.properties || [])
    .filter(p => !isSystemName(p.name))
    .map(p => ({ value: p.name, detail: [p.label, (p.edm_type || '').replace(/^Edm\./, '')].filter(Boolean).join(' · ') }));
}

function navCandidates(info) {
  return (info.nav_properties || [])
    .filter(n => !isSystemName(n.name))
    .map(n => ({ value: n.name, detail: 'navigation' }));
}

// Suggestions for `text` with the caret at `caret`. Returns
// { from, to, items: [{ value, detail, insert }] } — accepting an item
// replaces text[from..to) with `insert`.
export function suggestionsFor(mode, text, caret, info, version) {
  const none = { from: caret, to: caret, items: [] };
  if (!info) return none;
  const before = text.slice(0, caret);
  // Extend the token over identifier characters right of the caret too.
  const afterMatch = /^[\w/]*/.exec(text.slice(caret));
  const to = caret + (afterMatch ? afterMatch[0].length : 0);
  const word = /[\w/]*$/.exec(before)[0];
  const from = caret - word.length;
  const props = new Set((info.properties || []).map(p => p.name));
  const withInsert = (list, suffix = '') => list.map(c => ({ ...c, insert: c.value + (c.value.endsWith('(') || c.value.endsWith(' ') ? '' : suffix) }));

  if (mode === 'expand') {
    return { from, to, items: withInsert(rank(navCandidates(info), word)) };
  }
  if (mode === 'select') {
    return { from, to, items: withInsert(rank(propertyCandidates(info), word)) };
  }
  if (mode === 'orderby') {
    // Second word of a clause: `Prop asc|desc`.
    const clause = before.slice(before.lastIndexOf(',') + 1);
    if (/^\s*[\w/]+\s+\w*$/.test(clause)) {
      const dir = ['asc', 'desc'].map(v => ({ value: v, detail: 'direction' }));
      return { from, to, items: withInsert(rank(dir, word)) };
    }
    return { from, to, items: withInsert(rank(propertyCandidates(info), word)) };
  }
  if (mode === 'filter') {
    // Inside an open string literal: an odd number of quotes before the caret.
    if ((before.match(/'/g) || []).length % 2 === 1) return none;
    const head = before.slice(0, from);
    const prev = /([\w/']+|\))\s+$/.exec(head);
    if (prev) {
      const token = prev[1];
      if (props.has(token)) {
        const ops = COMPARISON.map(v => ({ value: v, detail: 'operator' }));
        return { from, to, items: withInsert(rank(ops, word), ' ') };
      }
      if (/['\d)]$/.test(token) || ['true', 'false', 'null'].includes(token)) {
        const ops = LOGICAL.map(v => ({ value: v, detail: 'logical' }));
        return { from, to, items: withInsert(rank(ops, word), ' ') };
      }
    }
    const fns = [version === 'V2' ? 'substringof(' : 'contains(', ...FUNCTIONS_COMMON]
      .map(v => ({ value: v, detail: 'function' }));
    if (!word) return none; // don't pop up on every space
    // A property at the top level is followed by an operator, so insert a
    // space (which chains into operator suggestions). Inside a function's
    // argument list (`startswith(Prop,`) it's followed by `,` or `)`.
    const openParens = (head.match(/\(/g) || []).length - (head.match(/\)/g) || []).length;
    const suffix = openParens > 0 ? '' : ' ';
    return { from, to, items: withInsert(rank([...propertyCandidates(info), ...fns], word), suffix) };
  }
  return none;
}

