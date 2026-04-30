// ── App-wide singleton state ──
//
// One mutable object that every module reads from and writes to via
// `state.X = ...`. The object pattern is deliberate: imported `let`
// bindings are read-only from the importer side, so a plain
// `export let currentProfile = null` would prevent other modules from
// reassigning. With a singleton-object export, every module sees the
// same live object and can mutate its properties directly.
//
// What lives here is *cross-module* state — fields touched by app.js
// today, soon by services/auth/results modules. Per-module private
// state (e.g. the value-list resolve cache, annotation-inspector
// caches, filter tooltip timer) stays in its own module.

export const state = {
  // Tab system (app.js owns the wiring; tabs.js will lift CRUD in batch 3).
  tabs: [],
  activeTabId: null,
  profileMap: new Map(),

  // Active-tab convenience mirrors. saveCurrentTabState() / restoreTabUI()
  // sync these with the active tab's per-tab fields when the user switches.
  currentProfile: null,
  currentServicePath: null,
  currentEntitySet: null,
  entitySets: [],
  cachedServices: null,
  lastSearchQuery: null,
  expandedDataStore: {},
  lastResultRows: null,

  // Persisted preference: SAP View toggles whether describe + results
  // surface typed annotation hints (HeaderInfo, Common.Text, etc.).
  // Initialised below from localStorage so refreshes preserve the choice.
  sapViewEnabled: false,
};

try {
  state.sapViewEnabled = localStorage.getItem('ox_sap_view_enabled') === '1';
} catch {
  /* ignore — SSR / private mode etc. */
}
