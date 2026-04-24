# SAP View

**SAP View** is the opt-in overlay that layers SAP/UI5 annotation awareness on top of the raw OData explorer. It turns the app from a generic OData client into a stack-bridge for ABAP backend ↔ Fiori frontend work: entity titles, description pairings, value-help pickers, pre-flight query validation, declared filter variants — all driven by the service's own `$metadata`.

It's off by default so the raw-data flow stays untouched. Toggle the **SAP View** pill in the status bar (bottom-right) to turn it on. The state is per-install, persisted in `localStorage`.

Everything below only lights up when SAP View is on (with one exception — annotation parsing happens always; only the UI effects are gated).

## Contents

- [Describe panel](#describe-panel)
- [Query bar](#query-bar)
- [Results grid](#results-grid)
- [Pre-flight validator](#pre-flight-validator)
- [Value-help picker (F4)](#value-help-picker-f4)
- [Annotation coverage](#annotation-coverage)
- [Deliberate gaps](#deliberate-gaps)

## Describe panel

Overlays on the per-property table that appears when you click an entity set:

- **Entity title subtitle** — `UI.HeaderInfo.TypeName` / `TypeNamePlural` render alongside the technical type name, so `WarehousePhysicalStockProductsType` reads as *"Manage Physical Stock - Product / Manage Physical Stock - Products"*.
- **"title" badge** on the property used as `UI.HeaderInfo.Title`.
- **Text-companion marker** — properties with `Common.Text` (V4) or `sap:text` (V2) show `↦ DescProp` next to the name.
- **Unit / currency markers** — `Measures.Unit` (or V2 `sap:unit`) renders as `↑ UnitProp`; `Measures.ISOCurrency` renders as `¤ CurrencyProp`.
- **Criticality dot** — `UI.Criticality` with a fixed level paints a colored dot (positive/critical/negative/info/neutral). `Path` variants show the source property.
- **Restriction pills** — `no filter`, `no sort`, `read-only`, `no create`, `no update`, `req.filter`, folded from `Capabilities.FilterRestrictions` / `SortRestrictions` / `InsertRestrictions` / `UpdateRestrictions` (V4) and `sap:filterable` / `sortable` / `creatable` / `updatable` / `required-in-filter` (V2).
- **`Common.FieldControl` pills** — `mandatory`, `read-only`, `n/a`, `hidden`, or `⇨ PathProp` for runtime-driven control.
- **`UI.Hidden` dim** — rows for properties Fiori wouldn't show drop to 60% opacity with muted text. Still visible and clickable.
- **`UI.HiddenFilter` pill** — `no filter UI`.
- **`sap:display-format` pill** — green badge `fmt: Date` / `fmt: NonNegative` / `fmt: UpperCase`.
- **F4 marker** (`⇒ F4`) — appears when the property has a value help (see [Value-help picker](#value-help-picker-f4)).
- **"biz key" marker** — `Common.SemanticKey` lists business-key properties (often distinct from the technical primary key, e.g. `Product` instead of a UUID). Listed properties get an amber "biz key" tag next to the name so scanning wide tables stays fast.
- **Semantic object pill (`↪ Name`)** — `Common.SemanticObject` declares a Fiori cross-app navigation target (e.g. `Product`, `Customer`). Hint-only for now; surfaces which properties would deep-link in a Fiori runtime.
- **"masked" pill** — `Common.Masked` marker on sensitive properties (PII, secrets, tax numbers). Amber warning style so you think twice before sharing screenshots or logs of the column.

## Query bar

- **Selection-fields chip bar** — `UI.SelectionFields` becomes a row of clickable chips above `$filter`. Clicking a chip appends `<chip> eq ''` to `$filter` with the cursor parked inside the quotes. Chips for `RequiredProperties` render amber so you see at a glance which ones the server will reject queries without.
- **"Fiori cols (N)" button** — next to `$select`. Populates `$select` with the column list from `UI.LineItem` (DataField `Value` paths, direct properties only). Augmented with `UI.PresentationVariant.RequestAtLeast` paths; when present the label shows `Fiori cols (N +M)` and the tooltip explains the augment. Also fills `$orderby` from `UI.PresentationVariant.SortOrder` when the service declares one — click gives you Fiori's full default view (columns + sort), not just the columns.
- **"Fiori filter" button** — next to "Fiori cols". Rebuilds `$filter` from a `UI.SelectionVariant`. Services that declare one "empty" variant first (e.g. a "Show All" with just a `Text`/`ID` and no `Parameters`/`SelectOptions`) would otherwise yield an unactionable button — the renderer walks the declared variants and picks the **first one with actual filter content**, so the click always produces something. The button label shows that variant's `Text`; a `+N` suffix signals how many other variants exist. Translation rules:
  - `Parameters` → `name eq <lit>`
  - `SelectOptions` ranges → per-operator translation (see [validator section](#pre-flight-validator) for the operator table)
  - A picker across all declared variants is a later enhancement.

## Results grid

When SAP View is on, the results table reshapes itself to look like a Fiori list report:

- **Column order** — declared `UI.LineItem` columns come first, in position order, then everything else from the response. Nested / expanded columns always go last.
- **Text-folded cells** — when a property has both `Common.Text` and `UI.TextArrangement`, the description folds into the ID column's cell:
  - `TextFirst` (Fiori default when unspecified) — `"Warehouse Berlin (WH01)"`
  - `TextLast` — `"WH01 (Warehouse Berlin)"`
  - `TextOnly` — just the description
  - `TextSeparate` — two columns, unfolded
  The raw ID stays in the cell's `data-cell-val` so click-to-filter uses the key, not the description.
- **`sap:display-format` applied** — `Date` strips the time portion (handles both V4 ISO 8601 and V2 `/Date(ms)/` format); `Time` keeps `HH:MM:SS`; `UpperCase` uppercases strings; `NonNegative` coerces negatives to `0`.
- **`UI.Criticality` dot** — cells whose property has a `Fixed` or `Path` criticality get a small colored dot prefix: green (Positive/3), amber (Critical/2), red (Negative/1), blue (Information/5), no dot for Neutral/0. `Path` criticality reads the numeric code from the companion property per row, so the color changes row-by-row. The raw cell value stays in `data-cell-val` (click-to-filter unaffected).

## Pre-flight validator

Before every query, the app cross-checks the URL against the service's declared restrictions. If any are violated, an amber warning strip appears above the results — the query **still runs** (server is the source of truth), but you see what the service thinks will fail.

Warnings fire on:

| Check | Source annotation |
|---|---|
| Property in `$filter` is non-filterable | `Capabilities.FilterRestrictions.NonFilterableProperties` / `sap:filterable=false` |
| Property in `$orderby` is non-sortable | `Capabilities.SortRestrictions.NonSortableProperties` / `sap:sortable=false` |
| Property marked `required-in-filter` missing from `$filter` | `Capabilities.FilterRestrictions.RequiredProperties` / `sap:required-in-filter` |
| `$top` set, service says `TopSupported=false` | `Capabilities.TopSupported` |
| `$skip` set, service says `SkipSupported=false` | `Capabilities.SkipSupported` |
| `$count` requested, service says `Countable=false` | `Capabilities.CountRestrictions.Countable` |
| `$expand` references a property on the non-expandable list | `Capabilities.ExpandRestrictions.NonExpandableProperties` |
| `$expand` used on a set with `Expandable=false` | `Capabilities.ExpandRestrictions.Expandable` |

## Value-help picker (F4)

Clicking the `⇒ F4` marker on a property opens a picker modal that fetches the referenced value-help entity set and lets you bind a picked row back into `$filter`. Three shapes are supported:

- **Inline `Common.ValueList`** — mapping lives on the service itself. Solid cyan marker.
- **`Common.ValueListReferences`** (S/4HANA pattern) — mapping lives in a separate F4 service. Dashed cyan marker. On open, the desktop resolves the relative URL against the current service path (preserving SAP matrix parameters like `;ps='...';va='...'`), fetches the referenced `$metadata`, and parses its `Common.ValueListMapping`. Results are cached per reference URL so reopens are instant.
- **`Common.ValueListWithFixedValues`** (marker-only) — muted gray marker. Clicking surfaces a status-bar hint; no picker (there's no mapping to drive).
- **V2 `sap:value-list="standard"` / `"fixed-values"`** — muted gray marker on V2 services that advertise a value help. V2 metadata doesn't carry the mapping (Fiori resolves by naming convention at runtime), so the picker isn't driveable from here — clicking surfaces a status-bar hint explaining the limitation.

Picker behavior:

- **Variant pills** — when the property declares multiple value helps (either multiple `Common.ValueList` annotations with different qualifiers, or multiple `ValueListReferences` URLs, or both), a pill bar at the top of the modal lets you switch between them. Inline variants render as `· label`, references as `↗ service-name`. Reference variants resolve lazily the first time they're selected; resolutions are cached per URL for instant reopen/switch. Single-source pickers hide the bar.
- **Pre-seeds its `$filter`** from the current main `$filter` — any `In` or `InOut` parameter whose local property is already pinned in the main filter gets echoed into the picker's filter. `Constant` parameters are always echoed.
- **`$search` input** — shown when the active ValueList says `SearchSupported=true`, *or* when the resolved F4 service declares `Capabilities.SearchRestrictions.Searchable=true` at the entity-set level (SAP's modern F4 services don't put the flag on the mapping record, so the resolver lifts it from the F4 `$metadata`). Press Enter to fetch with the search term applied; V4 emits `$search="term"`, V2 falls back to `search=term`.
- **Dynamic `$filter` placeholder** — uses the first `ValueListProperty` from the mapping as a hint (e.g. `startswith(EWMWarehouse,'HB')`), so you don't have to cross-reference the mapping line to guess the F4's column names.
- **Column order** prioritizes `ValueListProperty` names from the parameter mapping, then remaining keys.
- **On pick**: for every `InOut` and `Out` parameter with a local binding, writes `local_property eq <literal>` into the main `$filter`. Literals are quoted according to the local property's `edm_type` (`Edm.String` wrapped in single quotes, numerics raw, `Edm.Guid` as `guid'...'`, etc.). Clauses already present in the main filter are deduped.

## Annotation coverage

Compact status table. "Status" reflects what `SAP View` actually uses; parser-only support (captured in `ServiceMetadata.annotations` flat list but not typed) is implicit for everything else.

| Annotation | Effect | Status |
|---|---|---|
| `UI.HeaderInfo` | Entity title subtitle + title-column badge | ✅ |
| `Common.Text` / V2 `sap:text` | Describe marker + results-grid fold per TextArrangement | ✅ |
| `UI.TextArrangement` | Cell format `"text (id)"` / `"id (text)"` / text-only / separate | ✅ |
| `UI.Criticality` | Describe panel dot + results-grid cell dot (fixed uniform / path per-row) | ✅ |
| `Capabilities.FilterRestrictions` | `no filter` / `req.filter` pills + validator | ✅ |
| `Capabilities.SortRestrictions` | `no sort` pill + validator | ✅ |
| `Capabilities.InsertRestrictions` | `no create` pill | ✅ |
| `Capabilities.UpdateRestrictions` | `no update` pill | ✅ |
| `Capabilities.SearchRestrictions` | Drives `$search` input visibility in F4 picker | ✅ |
| `Capabilities.CountRestrictions` | `$count` validator | ✅ |
| `Capabilities.ExpandRestrictions` | `$expand` validator | ✅ |
| `Capabilities.TopSupported` | `$top` validator | ✅ |
| `Capabilities.SkipSupported` | `$skip` validator | ✅ |
| `Measures.Unit` / V2 `sap:unit` | `↑ UnitProp` marker | ✅ |
| `Measures.ISOCurrency` | `¤ CurrencyProp` marker | ✅ |
| `UI.SelectionFields` | Query-bar chip bar (amber when required-in-filter) | ✅ |
| `UI.LineItem` | "Fiori cols (N)" button + results column order | ✅ |
| `UI.PresentationVariant.RequestAtLeast` | Augments "Fiori cols" `$select` | ✅ |
| `UI.PresentationVariant.SortOrder` | Fills `$orderby` on "Fiori cols" click | ✅ |
| `UI.SelectionVariant` | "Fiori filter" button (picks first populated variant) | ✅ |
| `UI.SelectionPresentationVariant` | Inline SV/PV extracted; path references left to standalone SV/PV parse | ✅ |
| `Common.ValueList` (inline) | F4 picker, solid marker | ✅ |
| `Common.ValueListReferences` (S/4HANA) | F4 picker, dashed marker, resolved on open | ✅ |
| `Common.ValueListMapping` | Parsed inside referenced F4 services | ✅ |
| `Common.ValueListWithFixedValues` | Marker-only hint | ✅ |
| `Common.ValueList` qualifier variants | Multi-variant pill bar inside the picker; click switches active mapping | ✅ |
| `Common.FieldControl` | Pills (mandatory / read-only / n/a / hidden / path) | ✅ |
| `UI.Hidden` | Describe row dimmed | ✅ |
| `UI.HiddenFilter` | `no filter UI` pill | ✅ |
| `V2 sap:display-format` | Describe pill + results-grid formatting | ✅ |
| `V2 sap:filterable` / `sortable` / `creatable` / `updatable` / `required-in-filter` | Same pills as V4 Capabilities.* | ✅ |
| V2 `sap:value-list` marker (standard / fixed-values) | Muted F4 badge; click surfaces a "no mapping" hint (picker deferred until V2 convention-scan lands) | ✅ |
| `Common.SemanticKey` | "biz key" tag on listed properties | ✅ |
| `Common.SemanticObject` | `↪ Name` pill (cross-app nav hint) | ✅ |
| `Common.Masked` | "masked" amber warning pill | ✅ |

## Deliberate gaps

Things intentionally left out of SAP View for now, with the reasoning:

- **`UI.Facets` / `UI.FieldGroup` / `UI.Identification`** — the Object Page layout family. Implementing a "Fiori preview" mode is a large UI investment for low daily-troubleshooting payoff; keeping SAP View as an overlay on the explorer (not a Fiori runtime) is a design principle.
- **`edmx:Reference` / `edmx:IncludeAnnotations`** — external annotation-document loading. Theoretically elegant; the practical payoff (`Common.ValueListReferences`) is already handled. Other external-annotation cases are rare in SAP practice.
- **Results-grid criticality coloring** — `UI.Criticality = Path("StatusCriticality")` could color cells per row by looking up the linked column. Doable, but niche outside KPI services.
- **Raw annotation inspector panel** — a power-user view dumping `ServiceMetadata.annotations` grouped by namespace. Low UI cost (parser already captures them); on the backlog.
- **Fiori-readiness linter** — checklist flagging missing-but-expected annotations (no `HeaderInfo`, no `SelectionFields`, no `LineItem`). ABAP-dev-facing feature; on the backlog.

## Parser-derived JSON (CLI)

Running `sap-odata describe <EntitySet> --json` on the CLI surfaces all of the above as structured fields on the entity type and its properties — see [CLI-REFERENCE.md](./CLI-REFERENCE.md) for the full schema. Handy for scripting linting or comparing services across environments without re-parsing `$metadata`.
