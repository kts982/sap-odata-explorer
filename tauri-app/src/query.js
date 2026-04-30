// ── Query helpers + selection filter bar ──
//
// Three concerns:
//   - SAP-view describe-row decorations (propertyFlagHints) — small
//     pills surfacing sap:filterable=false / FieldControl / Hidden /
//     etc. so the user spots restrictions at a glance.
//   - Pre-flight query validation against typed annotations
//     (validateQueryRestrictions + showSapViewWarnings) — surfaces
//     "the server will reject this $orderby" / "$top is unsupported"
//     before the request is sent.
//   - The mock Fiori filter bar — one row per UI.SelectionFields
//     property with operator dropdown + value input. Apply builds an
//     OData $filter expression and drops it into the main query bar.
//
// Pure module — no circular imports back to app.js.

import { state } from './state.js';
import { escapeHtml } from './html.js';
import { formatODataLiteral } from './format.js';
import { setStatus } from './status.js';
import { getActiveTab } from './tabs.js';

// SAP view helper: render small pills for property-level restrictions.
// We only surface *deviations from the default* — filterable/sortable/
// creatable/updatable are normally true, so showing "no filter", "no sort",
// "read-only" is what's informative. required_in_filter=true is also
// visible because it constrains how the user writes $filter.
export function propertyFlagHints(p) {
  const badges = [];
  if (p.filterable === false) {
    badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="sap:filterable=false — server rejects $filter on this column">no filter</span>`);
  }
  if (p.sortable === false) {
    badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="sap:sortable=false — server rejects $orderby on this column">no sort</span>`);
  }
  if (p.creatable === false && p.updatable === false) {
    badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="sap:creatable=false and sap:updatable=false — server assigns this value, clients cannot write it">read-only</span>`);
  } else {
    if (p.creatable === false) {
      badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="sap:creatable=false">no create</span>`);
    }
    if (p.updatable === false) {
      badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="sap:updatable=false">no update</span>`);
    }
  }
  if (p.required_in_filter === true) {
    badges.push(`<span class="text-[9px] text-ox-amber bg-ox-amberGlow border border-ox-amber/40 rounded-sm px-1 py-px" title="sap:required-in-filter=true — the server requires $filter to constrain this column">req.filter</span>`);
  }
  // Common.FieldControl — write/display control. Mandatory overlaps
  // semantically with required_in_filter so we keep the pills distinct
  // (one is $filter-side, the other is write-side). ReadOnly overlaps
  // with updatable=false; suppress the pill when we'd double-count.
  if (p.field_control) {
    const fc = p.field_control;
    if (fc.kind === 'mandatory') {
      badges.push(`<span class="text-[9px] text-ox-amber bg-ox-amberGlow border border-ox-amber/40 rounded-sm px-1 py-px" title="Common.FieldControl=Mandatory — required on write">mandatory</span>`);
    } else if (fc.kind === 'readonly' && !(p.updatable === false && p.creatable === false)) {
      badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="Common.FieldControl=ReadOnly">read-only</span>`);
    } else if (fc.kind === 'inapplicable') {
      badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="Common.FieldControl=Inapplicable — not relevant for this record">n/a</span>`);
    } else if (fc.kind === 'hidden') {
      badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="Common.FieldControl=Hidden">hidden</span>`);
    } else if (fc.kind === 'path') {
      badges.push(`<span class="text-[9px] text-ox-blue border border-ox-blue/40 rounded-sm px-1 py-px" title="Common.FieldControl Path — state driven by ${escapeHtml(fc.value)} at runtime">⇨ ${escapeHtml(fc.value)}</span>`);
    }
    // `optional` is the default; no pill needed.
  }
  // UI.Hidden / UI.HiddenFilter — marker pills.
  if (p.hidden && (!p.field_control || p.field_control.kind !== 'hidden')) {
    badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="UI.Hidden — Fiori would not show this property">UI hidden</span>`);
  }
  if (p.hidden_filter) {
    badges.push(`<span class="text-[9px] text-ox-muted bg-ox-panel border border-ox-border rounded-sm px-1 py-px" title="UI.HiddenFilter — shown as a column but suppressed from Fiori's filter bar">no filter UI</span>`);
  }
  // V2 sap:display-format — presentation hint. Small-caps pill.
  if (p.display_format) {
    const val = p.display_format;
    badges.push(`<span class="text-[9px] text-ox-green border border-ox-green/40 rounded-sm px-1 py-px" title="sap:display-format=${escapeHtml(val)}">fmt: ${escapeHtml(val)}</span>`);
  }
  // Common.SemanticObject — Fiori cross-app navigation target.
  if (p.semantic_object) {
    badges.push(`<span class="text-[9px] text-ox-blue border border-ox-blue/40 rounded-sm px-1 py-px" title="Common.SemanticObject — Fiori cross-app navigation target">&#8605; ${escapeHtml(p.semantic_object)}</span>`);
  }
  // Common.Masked — sensitive data warning.
  if (p.masked) {
    badges.push(`<span class="text-[9px] text-ox-amber bg-ox-amberGlow border border-ox-amber/40 rounded-sm px-1 py-px" title="Common.Masked — sensitive / PII data; Fiori masks the value at runtime">masked</span>`);
  }
  return badges.length ? ' ' + badges.join(' ') : '';
}

// Extract identifier-looking tokens from a free-form OData expression.
// Good enough for cross-checking against known property names — any
// identifier that happens to appear in the expression AND matches a
// restricted property name flags as a likely reference.
export function extractIdentifiers(text) {
  if (!text) return [];
  return (text.match(/[A-Za-z_][A-Za-z0-9_]*/g) || []);
}

// Pre-flight validator for SAP View. Returns a list of human-readable
// restriction violations — empty list means "OK to run".
export function validateQueryRestrictions(params, info) {
  if (!info || !Array.isArray(info.properties)) return [];
  const issues = [];
  const byName = new Map(info.properties.map(p => [p.name, p]));

  if (params.filter) {
    const tokens = new Set(extractIdentifiers(params.filter));
    for (const p of info.properties) {
      if (p.filterable === false && tokens.has(p.name)) {
        issues.push(`'${p.name}' is non-filterable (Capabilities.FilterRestrictions / sap:filterable=false) but referenced in $filter.`);
      }
    }
  }

  if (params.orderby) {
    const tokens = new Set(extractIdentifiers(params.orderby));
    for (const p of info.properties) {
      if (p.sortable === false && tokens.has(p.name)) {
        issues.push(`'${p.name}' is non-sortable but referenced in $orderby.`);
      }
    }
  }

  // required_in_filter: these properties MUST be narrowed in $filter.
  const required = info.properties.filter(p => p.required_in_filter === true);
  if (required.length) {
    const tokens = new Set(extractIdentifiers(params.filter || ''));
    for (const p of required) {
      if (!tokens.has(p.name)) {
        issues.push(`'${p.name}' requires a filter clause (Capabilities.FilterRestrictions.RequiredProperties / sap:required-in-filter).`);
      }
    }
  }

  // Entity-set-level capabilities — the server will 500 on these if
  // the declared flag is explicitly false. Only the `false` case is
  // informative; `None`/unset defaults to "supported".
  if ((params.top !== null && params.top !== undefined && params.top !== '') && info.top_supported === false) {
    issues.push('$top was set, but Capabilities.TopSupported=false on this set — the server will reject pagination.');
  }
  if ((params.skip !== null && params.skip !== undefined && params.skip !== '') && info.skip_supported === false) {
    issues.push('$skip was set, but Capabilities.SkipSupported=false on this set — the server will reject pagination.');
  }
  if (params.count === true && info.countable === false) {
    issues.push('$count requested, but Capabilities.CountRestrictions.Countable=false on this set.');
  }

  // $expand: flag nav paths the service marked non-expandable. We only
  // match on the first segment (the nav prop directly on the annotated
  // entity) — multi-hop bans would require walking the type graph.
  if (params.expand && Array.isArray(info.non_expandable_properties) && info.non_expandable_properties.length) {
    const expandRoots = new Set(
      params.expand.split(',').map(s => s.trim().split('/')[0]).filter(Boolean)
    );
    for (const np of info.non_expandable_properties) {
      if (expandRoots.has(np)) {
        issues.push(`'${np}' is listed in Capabilities.ExpandRestrictions.NonExpandableProperties but referenced in $expand.`);
      }
    }
  }
  if (params.expand && info.expandable === false) {
    issues.push('$expand requested, but Capabilities.ExpandRestrictions.Expandable=false — the set rejects expansion entirely.');
  }

  // byName lookup silences unused warnings; also handy for future checks.
  void byName;
  return issues;
}

// Show or hide the amber warnings strip above the results. An empty
// list hides it. Called by executeQuery and by the SAP-view toggle so
// stale warnings don't linger after the user flips the mode off.
export function showSapViewWarnings(issues) {
  const strip = document.getElementById('sapViewWarnings');
  const list = document.getElementById('sapViewWarningsList');
  if (!strip || !list) return;
  if (!issues || issues.length === 0) {
    strip.classList.add('hidden');
    list.textContent = '';
    return;
  }
  strip.classList.remove('hidden');
  list.textContent = issues.map(i => `• ${i}`).join('\n');
}

// Render the "selection fields" chip bar above the query inputs. Only
// visible when SAP View is on, the entity type has UI.SelectionFields,
// and we're looking at that entity's describe panel. Clicking a chip
// seeds $filter with a skeleton clause the user can complete.
export function renderSelectionFieldsBar(info) {
  const bar = document.getElementById('selectionFieldsBar');
  const host = document.getElementById('selectionFieldsChips');
  if (!bar || !host) return;
  const fields = state.sapViewEnabled && info && Array.isArray(info.selection_fields)
    ? info.selection_fields
    : [];
  if (fields.length === 0) {
    bar.classList.add('hidden');
    host.innerHTML = '';
    return;
  }
  bar.classList.remove('hidden');
  // Amber-flag chips whose backing property is required-in-filter, so the
  // user sees at a glance which selection fields the server will reject
  // queries without.
  const byName = new Map(
    Array.isArray(info.properties)
      ? info.properties.map(p => [p.name, p])
      : []
  );
  host.innerHTML = fields
    .map(name => {
      const p = byName.get(name);
      const req = p && p.required_in_filter === true;
      const cls = req
        ? 'text-[10px] px-1.5 py-0.5 rounded-sm text-ox-amber bg-ox-amberGlow border border-ox-amber/40 hover:bg-ox-amber/20'
        : 'btn-ghost text-[10px] px-1.5 py-0.5 rounded-sm';
      const tipBase = req
        ? 'Required in $filter — append and narrow'
        : 'Append to $filter';
      const title = `${tipBase}\nShift-click to append to $select instead`;
      return `<button type="button" class="${cls}" data-action="selection-field" data-name="${escapeHtml(name)}" title="${title}">${escapeHtml(name)}</button>`;
    })
    .join('');
}

// ══════════════════════════════════════════════════════════════
// ── SELECTION FILTER BAR ──
// ══════════════════════════════════════════════════════════════
// Mock Fiori filter bar: one row per UI.SelectionFields property with
// an operator dropdown + value input. Apply builds a $filter clause
// and drops it into the query bar's qFilter. Useful when the user
// wants to fill several selection fields without memorizing OData
// operator syntax (startswith/contains/between).

export function openFilterBar() {
  const tab = getActiveTab();
  const info = tab && tab._lastDescribeInfo;
  if (!info || !Array.isArray(info.selection_fields) || info.selection_fields.length === 0) {
    setStatus('This entity declares no UI.SelectionFields.');
    return;
  }
  const modal = document.getElementById('filterBarModal');
  const rowsHost = document.getElementById('fbRows');
  const subtitle = document.getElementById('fbSubtitle');
  subtitle.textContent = info.name;
  rowsHost.innerHTML = buildFilterBarRows(info);
  modal.classList.remove('hidden');
  // Focus the first value input.
  setTimeout(() => {
    const first = rowsHost.querySelector('input[data-fb="value"]');
    if (first) first.focus();
  }, 0);
}

export function closeFilterBar() {
  document.getElementById('filterBarModal').classList.add('hidden');
}

// Render one row per SelectionField. Each row is a flex with label,
// operator dropdown, value input, and (for `between`) a second value
// input. Operators cover strings, numbers, and dates; we don't filter
// by type because SAP's CDS sometimes annotates columns in ways that
// make the "right" set hard to guess.
export function buildFilterBarRows(info) {
  const propByName = new Map(info.properties.map(p => [p.name, p]));
  const operators = [
    ['eq', '='], ['ne', '≠'],
    ['gt', '>'], ['ge', '≥'], ['lt', '<'], ['le', '≤'],
    ['contains', 'contains'],
    ['startswith', 'starts with'], ['endswith', 'ends with'],
    ['between', 'between'],
  ];
  const opOptions = operators
    .map(([v, label]) => `<option value="${v}">${label}</option>`)
    .join('');
  return info.selection_fields.map(name => {
    const p = propByName.get(name);
    const type = p ? p.edm_type.replace('Edm.', '') : '';
    const req = p && p.required_in_filter === true;
    const reqBadge = req
      ? `<span class="text-[9px] text-ox-amber bg-ox-amberGlow border border-ox-amber/40 rounded-sm px-1 ml-1" title="required-in-filter">req</span>`
      : '';
    return `
      <div class="grid grid-cols-[160px_90px_1fr_auto] gap-2 items-center" data-fb-row data-field="${escapeHtml(name)}">
        <div class="text-[11px] text-ox-text truncate" title="${escapeHtml(name)} (${escapeHtml(type)})">${escapeHtml(name)}${reqBadge} <span class="text-ox-dim">${escapeHtml(type)}</span></div>
        <select data-fb="op" class="bg-ox-surface text-ox-text text-[11px] font-mono border border-ox-border rounded-sm px-1.5 py-1 outline-hidden">${opOptions}</select>
        <div data-fb="inputs" class="flex items-center gap-1">
          <input data-fb="value" type="text" placeholder="value"
            class="flex-1 bg-ox-surface text-ox-text text-xs font-mono border border-ox-border rounded-sm px-2 py-1 outline-hidden" />
        </div>
        <button type="button" data-action="fb-clear-row" class="btn-ghost text-[10px] px-1.5 py-0.5 rounded-sm" title="Clear this row">×</button>
      </div>`;
  }).join('');
}

export function resetFilterBar() {
  const host = document.getElementById('fbRows');
  host.querySelectorAll('input[data-fb="value"], input[data-fb="value-high"]').forEach(i => i.value = '');
  host.querySelectorAll('select[data-fb="op"]').forEach(s => s.value = 'eq');
  // Remove any "high" inputs that may have been added by between-operator.
  host.querySelectorAll('[data-fb="value-high"]').forEach(el => el.remove());
}

// Switch the row's inputs area to show a second value when operator
// is `between`. Revert to single input for anything else.
export function onFilterBarOpChange(selectEl) {
  const row = selectEl.closest('[data-fb-row]');
  if (!row) return;
  const inputs = row.querySelector('[data-fb="inputs"]');
  const op = selectEl.value;
  const hasHigh = inputs.querySelector('[data-fb="value-high"]');
  if (op === 'between' && !hasHigh) {
    const sep = document.createElement('span');
    sep.className = 'text-ox-dim text-[10px]';
    sep.textContent = 'and';
    sep.setAttribute('data-fb', 'value-high-sep');
    const high = document.createElement('input');
    high.type = 'text';
    high.placeholder = 'upper';
    high.className = 'flex-1 bg-ox-surface text-ox-text text-xs font-mono border border-ox-border rounded-sm px-2 py-1 outline-hidden';
    high.setAttribute('data-fb', 'value-high');
    inputs.appendChild(sep);
    inputs.appendChild(high);
  } else if (op !== 'between' && hasHigh) {
    inputs.querySelectorAll('[data-fb="value-high"], [data-fb="value-high-sep"]').forEach(el => el.remove());
  }
}

// Walk the filter-bar rows and build an OData $filter expression.
// Empty rows are skipped. Literals are quoted via formatODataLiteral
// using the property's edm_type. Clauses are joined with `and`.
export function buildFilterBarExpression() {
  const tab = getActiveTab();
  const info = tab && tab._lastDescribeInfo;
  if (!info) return '';
  const propByName = new Map(info.properties.map(p => [p.name, p]));
  const clauses = [];
  const rows = document.querySelectorAll('#fbRows [data-fb-row]');
  rows.forEach(row => {
    const name = row.dataset.field;
    const prop = propByName.get(name);
    if (!prop) return;
    const op = row.querySelector('[data-fb="op"]').value;
    const val = row.querySelector('[data-fb="value"]').value.trim();
    if (val === '') return;
    const lit = formatODataLiteral(val, prop.edm_type);
    let clause;
    switch (op) {
      case 'eq': case 'ne': case 'gt': case 'ge': case 'lt': case 'le':
        clause = `${name} ${op} ${lit}`;
        break;
      case 'contains':
        clause = `contains(${name},${lit})`;
        break;
      case 'startswith':
        clause = `startswith(${name},${lit})`;
        break;
      case 'endswith':
        clause = `endswith(${name},${lit})`;
        break;
      case 'between': {
        const high = row.querySelector('[data-fb="value-high"]');
        const highVal = high ? high.value.trim() : '';
        if (!highVal) {
          clause = `${name} ge ${lit}`; // degraded form — no upper bound given
        } else {
          const highLit = formatODataLiteral(highVal, prop.edm_type);
          clause = `(${name} ge ${lit} and ${name} le ${highLit})`;
        }
        break;
      }
      default:
        return;
    }
    clauses.push(clause);
  });
  return clauses.join(' and ');
}

export function applyFilterBar() {
  const expr = buildFilterBarExpression();
  if (!expr) {
    setStatus('No filter rows with values — nothing applied.');
    return;
  }
  document.getElementById('qFilter').value = expr;
  closeFilterBar();
  document.getElementById('qFilter').focus();
}
