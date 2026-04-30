// ── Fiori-style query helpers ──
//
// "Fiori cols" / "Fiori filter" / Fiori-readiness panel — surfaces
// SAP-annotation-driven shortcuts that mirror what a Fiori list-report
// would render. All visibility checks gate on state.sapViewEnabled so
// the buttons stay hidden when the user is in raw-EDM mode.
//
// Pure module — no circular imports.

import { state } from './state.js';
import { safeHtml, raw } from './html.js';
import { formatODataLiteral } from './format.js';
import { getActiveTab } from './tabs.js';

// Show the "Fiori cols" button when SAP View is on and UI.LineItem is
// present. Filter to DataFields whose value_path is an actual property
// on this entity so V4 $select stays valid — nav-path DataFields are
// skipped (they belong in $expand).
export function renderFioriColsButton(info) {
  const btn = document.getElementById('btnFioriCols');
  if (!btn) return;
  const active = state.sapViewEnabled && info;
  const fields = active && Array.isArray(info.line_item) ? info.line_item : [];
  const propNames = new Set(
    Array.isArray(info && info.properties) ? info.properties.map(p => p.name) : []
  );
  // LineItem value_paths (the visible columns Fiori would show).
  const linePaths = fields
    .map(f => f && f.value_path)
    .filter(p => typeof p === 'string' && propNames.has(p));
  // PresentationVariant.RequestAtLeast (the silent "always include" set
  // — time zones, description keys, etc.). Appended AFTER the LineItem
  // columns so they don't disturb the positional order a Fiori list
  // report would use. Dedupe against line_item to avoid double-listing.
  const lineSet = new Set(linePaths);
  const requestAtLeast = active && Array.isArray(info.request_at_least)
    ? info.request_at_least.filter(p => propNames.has(p) && !lineSet.has(p))
    : [];
  const paths = [...linePaths, ...requestAtLeast];
  if (paths.length === 0) {
    btn.classList.add('hidden');
    btn.removeAttribute('data-paths');
    btn.removeAttribute('data-orderby');
    return;
  }
  btn.classList.remove('hidden');
  // Label notes the RequestAtLeast augment when it kicked in so the
  // user understands why the select is wider than the visible cols.
  const suffix = requestAtLeast.length ? ` +${requestAtLeast.length}` : '';
  btn.textContent = `Fiori cols (${linePaths.length}${suffix})`;
  // UI.PresentationVariant.SortOrder → "$orderby" string. Built here so
  // the click handler can drop it in without re-reading the describe
  // info. Only direct properties survive (nav-path sorts would need
  // $expand gymnastics).
  const sortOrder = active && Array.isArray(info.sort_order) ? info.sort_order : [];
  const orderbyClauses = sortOrder
    .filter(s => s && typeof s.property === 'string' && propNames.has(s.property))
    .map(s => `${s.property} ${s.descending ? 'desc' : 'asc'}`);
  const orderbyStr = orderbyClauses.join(',');
  const tipBase = 'Populate $select with UI.LineItem default columns (Fiori list report).';
  const tipLines = [tipBase];
  if (requestAtLeast.length) {
    tipLines.push(`Includes ${requestAtLeast.length} UI.PresentationVariant.RequestAtLeast field(s).`);
  }
  if (orderbyStr) {
    tipLines.push(`Also sets $orderby: ${orderbyStr}`);
  }
  btn.title = tipLines.join('\n');
  btn.dataset.paths = paths.join(',');
  btn.dataset.orderby = orderbyStr;
}

// Show the "Fiori filter" button when SAP View is on and the entity
// has at least one UI.SelectionVariant declared. We use the default
// (no-qualifier) variant for the actual filter build; the button's
// label hints at the qualified-variant count so the user knows extras
// exist (multi-variant picker UI can come later).
export function renderFioriFilterButton(info) {
  const btn = document.getElementById('btnFioriFilter');
  if (!btn) return;
  const variants = state.sapViewEnabled && info && Array.isArray(info.selection_variants)
    ? info.selection_variants
    : [];
  if (variants.length === 0) {
    btn.classList.add('hidden');
    btn.removeAttribute('data-variant-index');
    return;
  }
  // Pick the first variant that actually builds a non-empty $filter.
  // Services sometimes declare an empty "default view" variant (e.g.
  // "Show All") first and only put real filter clauses on qualified
  // variants — preferring a populated one gives the user a useful
  // result on the single click.
  let chosenIndex = -1;
  let chosenClause = '';
  for (let i = 0; i < variants.length; i++) {
    const clause = buildSelectionVariantFilter(variants[i], info);
    if (clause) {
      chosenIndex = i;
      chosenClause = clause;
      break;
    }
  }
  if (chosenIndex === -1) {
    // None of the variants produced an actionable filter — hide
    // rather than offer a dead button.
    btn.classList.add('hidden');
    return;
  }
  const variant = variants[chosenIndex];
  btn.classList.remove('hidden');
  const totalCount = variants.length;
  const label = variant.text
    ? `Fiori filter: ${variant.text}`
    : 'Fiori filter';
  const suffix = totalCount > 1 ? ` (+${totalCount - 1})` : '';
  btn.textContent = label + suffix;
  const preview = chosenClause.length > 80 ? chosenClause.slice(0, 80) + '…' : chosenClause;
  const extraLines = [];
  if (totalCount > 1) {
    extraLines.push(`${totalCount - 1} additional variant(s) not yet exposed in a picker.`);
  }
  if (variant.qualifier) {
    extraLines.push(`Qualifier: ${variant.qualifier}`);
  }
  btn.title = `$filter ← ${preview}${extraLines.length ? '\n\n' + extraLines.join('\n') : ''}`;
  btn.dataset.variantIndex = String(chosenIndex);
}

// Replace $filter with the clause built from a UI.SelectionVariant.
// Overwrites rather than merges (same reasoning as "Fiori cols" — the
// action's meaning is "show me this variant's filter as-is"). Uses the
// variant index stashed by renderFioriFilterButton so an empty leading
// variant doesn't get picked over a populated qualified one.
export function applyFioriFilter() {
  const tab = getActiveTab();
  const info = tab && tab._lastDescribeInfo;
  if (!info || !Array.isArray(info.selection_variants) || info.selection_variants.length === 0) return;
  const btn = document.getElementById('btnFioriFilter');
  const idx = btn && btn.dataset.variantIndex
    ? parseInt(btn.dataset.variantIndex, 10)
    : 0;
  const variant = info.selection_variants[idx] || info.selection_variants[0];
  const clause = buildSelectionVariantFilter(variant, info);
  if (!clause) return;
  const input = document.getElementById('qFilter');
  input.value = clause;
  input.focus();
}

// Convert a SelectionVariant into an OData $filter expression:
//   - Parameters become `name eq <lit>` clauses (AND-joined with the rest).
//   - Each SelectOption becomes an OR-joined group of range clauses for
//     that property, optionally wrapped in `not (...)` for sign=E.
//   - Properties are AND-joined overall.
// Returns an empty string when the variant has no usable clauses.
export function buildSelectionVariantFilter(variant, info) {
  if (!variant) return '';
  const propByName = new Map(
    Array.isArray(info && info.properties) ? info.properties.map(p => [p.name, p]) : []
  );
  const andParts = [];
  for (const param of variant.parameters || []) {
    const prop = propByName.get(param.property_name);
    if (!prop) continue;
    const lit = formatODataLiteral(param.property_value, prop.edm_type);
    andParts.push(`${param.property_name} eq ${lit}`);
  }
  for (const opt of variant.select_options || []) {
    const prop = propByName.get(opt.property_name);
    if (!prop) continue;
    const rangeClauses = [];
    for (const range of opt.ranges || []) {
      const clause = rangeToFilter(prop, range);
      if (clause) rangeClauses.push(clause);
    }
    if (rangeClauses.length === 0) continue;
    const combined = rangeClauses.length === 1
      ? rangeClauses[0]
      : `(${rangeClauses.join(' or ')})`;
    andParts.push(combined);
  }
  return andParts.join(' and ');
}

// One SelectionRange → one OData filter clause. Handles the seven
// common operators; CP/NP (SAP SELECT-OPTIONS pattern matching with *)
// are skipped because their OData translation depends on server-side
// substringof/contains support and the wildcard syntax mismatch — not
// worth mis-rendering for an MVP.
export function rangeToFilter(prop, range) {
  const lit = formatODataLiteral(range.low, prop.edm_type);
  const name = prop.name;
  let clause;
  switch (range.option) {
    case 'eq': clause = `${name} eq ${lit}`; break;
    case 'ne': clause = `${name} ne ${lit}`; break;
    case 'gt': clause = `${name} gt ${lit}`; break;
    case 'ge': clause = `${name} ge ${lit}`; break;
    case 'lt': clause = `${name} lt ${lit}`; break;
    case 'le': clause = `${name} le ${lit}`; break;
    case 'bt': {
      if (range.high === null || range.high === undefined) return '';
      const hi = formatODataLiteral(range.high, prop.edm_type);
      clause = `(${name} ge ${lit} and ${name} le ${hi})`;
      break;
    }
    case 'nb': {
      if (range.high === null || range.high === undefined) return '';
      const hi = formatODataLiteral(range.high, prop.edm_type);
      // Invert BT: outside the closed interval.
      clause = `(${name} lt ${lit} or ${name} gt ${hi})`;
      break;
    }
    default:
      // CP/NP and anything unknown — skip rather than guess.
      return '';
  }
  return range.sign === 'e' ? `not (${clause})` : clause;
}

// Replace $select (and $orderby if the service declares one) with the
// Fiori LineItem defaults. Overwrites rather than appends — "show me
// what Fiori shows" means both the column list and the default sort.
export function applyFioriCols() {
  const btn = document.getElementById('btnFioriCols');
  const input = document.getElementById('qSelect');
  if (!btn || !input) return;
  const paths = (btn.dataset.paths || '').split(',').filter(Boolean);
  if (paths.length === 0) return;
  input.value = paths.join(',');
  // UI.PresentationVariant.SortOrder → $orderby. Stashed on the button
  // as `data-orderby` by renderFioriColsButton so the click is a pure
  // apply step with no DOM lookups.
  const orderby = btn.dataset.orderby || '';
  const orderbyInput = document.getElementById('qOrderby');
  if (orderbyInput) orderbyInput.value = orderby;
  input.focus();
}

// Fiori-readiness checklist, rendered below the describe tables when
// SAP View is on. Shows the parser's findings with a traffic-light
// dot per row and groups them by category. Summary counts up-front
// so the user can tell "is this service shaped like Fiori expects?"
// at a glance.
export function renderFioriReadinessPanel(info) {
  const findings = Array.isArray(info.fiori_readiness) ? info.fiori_readiness : [];
  if (findings.length === 0) return '';
  const counts = { pass: 0, warn: 0, miss: 0 };
  for (const f of findings) {
    if (counts[f.severity] !== undefined) counts[f.severity]++;
  }
  const summary = [
    counts.pass ? safeHtml`<span class="text-ox-green">&#9679; ${counts.pass} pass</span>` : '',
    counts.warn ? safeHtml`<span class="text-ox-amber">&#9679; ${counts.warn} warn</span>` : '',
    counts.miss ? safeHtml`<span class="text-ox-red">&#9679; ${counts.miss} miss</span>` : '',
  ].filter(Boolean).join(' <span class="text-ox-border">·</span> ');
  // Group by category, preserve original order within each group.
  const order = ['profile', 'identity', 'listreport', 'filtering', 'fields', 'integrity', 'capabilities'];
  const byCategory = new Map(order.map(k => [k, []]));
  for (const f of findings) {
    if (!byCategory.has(f.category)) byCategory.set(f.category, []);
    byCategory.get(f.category).push(f);
  }
  const pretty = {
    profile: 'Profile',
    identity: 'Identity',
    listreport: 'List report',
    filtering: 'Filtering',
    fields: 'Fields',
    integrity: 'Integrity',
    capabilities: 'Capabilities',
  };
  const categoryBlocks = [];
  for (const [cat, items] of byCategory) {
    if (!items || items.length === 0) continue;
    const rows = items.map(f => {
      const color = f.severity === 'pass' ? 'text-ox-green'
        : f.severity === 'warn' ? 'text-ox-amber'
        : 'text-ox-red';
      let extra = '';
      // ABAP CDS "fix hint" — surfaces the annotation to add at the
      // source so the linter teaches, not just grades. Only present
      // on actionable (warn/miss) findings; passes skip this line.
      if (f.suggested_cds || f.why_in_fiori) {
        const parts = [];
        if (f.suggested_cds) {
          parts.push(safeHtml`<span class="text-ox-blue font-mono">ABAP CDS:</span> <code class="text-ox-blue">${f.suggested_cds}</code>`);
        }
        if (f.why_in_fiori) {
          parts.push(safeHtml`<div class="text-ox-dim">${f.why_in_fiori}</div>`);
        }
        extra = safeHtml`<div class="mt-1 text-[10px] text-ox-muted leading-snug">${raw(parts.join(''))}</div>`;
      }
      return safeHtml`
        <div class="px-3 py-1 border-t border-ox-border/40 flex items-start gap-2 text-[11px]">
          <span class="${color} mt-0.5">&#9679;</span>
          <div class="flex-1">
            <span class="text-ox-dim font-mono">${f.code}</span> — <span class="text-ox-text">${f.message}</span>
            ${raw(extra)}
          </div>
        </div>`;
    }).join('');
    categoryBlocks.push(safeHtml`
      <div class="px-3 py-1 bg-ox-surface/40 text-[9px] uppercase tracking-widest text-ox-muted border-t border-ox-border/40">${pretty[cat] || cat}</div>
      ${raw(rows)}`);
  }
  return safeHtml`
    <div class="mt-4 border border-ox-border rounded-sm overflow-hidden">
      <div class="px-3 py-1.5 bg-ox-panel text-[10px] uppercase tracking-widest text-ox-dim flex items-center gap-3">
        <span class="font-medium">Fiori readiness</span>
        <span class="text-[10px] normal-case tracking-normal">${raw(summary)}</span>
      </div>
      ${raw(categoryBlocks.join(''))}
    </div>`;
}
