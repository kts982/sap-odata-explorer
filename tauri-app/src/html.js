// ── HTML escaping + safe template helper ──
//
// safeHtml is a tagged-template helper for HTML strings destined for
// innerHTML. Every `${...}` interpolation is escaped via escapeHtml unless
// wrapped with `raw(...)` to opt in to pre-escaped HTML.
//
// Convention: any backtick template literal that ends up assigned to
// innerHTML MUST be tagged with safeHtml. CI lints for `\.innerHTML\s*=\s*\`
// to fail the build on bare template literals; the only allowed forms are:
//   el.innerHTML = '<static>...';             // single/double-quoted string
//   el.innerHTML = safeHtml`<tpl>${x}</tpl>`; // tagged template
//   el.innerHTML = preBuiltSafeHtmlString;    // assigned variable (build with safeHtml)
//
// The raw-HTML opt-out marker is a closure-private Symbol rather than a
// regular property: JSON cannot synthesise Symbol-keyed properties, so an
// untrusted server payload (e.g. `{"__rawHtml": true, ...}`) cannot forge
// a marker and bypass escaping. Only code with a reference to
// RAW_HTML_MARKER — i.e. callers of `raw()` — can produce a tagged value.

export function escapeHtml(str) {
  const div = document.createElement('div');
  div.textContent = str;
  return div.innerHTML;
}

const RAW_HTML_MARKER = Symbol('safeHtmlRaw');

export function safeHtml(strings, ...values) {
  let out = strings[0];
  for (let i = 0; i < values.length; i++) {
    const v = values[i];
    if (v && typeof v === 'object' && v[RAW_HTML_MARKER] === true) {
      out += v.value;
    } else {
      out += escapeHtml(v == null ? '' : String(v));
    }
    out += strings[i + 1];
  }
  return out;
}

// Mark a string as already-safe HTML so safeHtml passes it through
// unescaped. Use ONLY for HTML you trust to be already escaped (typically
// built via safeHtml itself, or a static literal known not to contain
// untrusted data).
export function raw(htmlString) {
  return {
    [RAW_HTML_MARKER]: true,
    value: htmlString == null ? '' : String(htmlString),
  };
}
