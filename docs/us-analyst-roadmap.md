# HookEcho — U.S. Analyst-Grade Roadmap

> **Purpose:** implementation roadmap for Claude Code, Codex, and human contributors.
>
> **Scope:** United States weather analysis only. Do **not** spend roadmap effort on international radar, international model coverage, or international warning products. Existing international code may remain working, but it is outside this roadmap.
>
> **Relationship to `ROADMAP.md`:** this file is additive. Do not replace or delete the existing roadmap. This document is the detailed path from the current HookEcho feature set to a top-tier U.S. meteorological analysis workstation comparable in breadth to RadarOmega / WeatherFront / RadarScope, in low-latency presentation to WeatherWise / WSV3, and in radar-analysis depth to GR2Analyst.
>
> **Snapshot date:** 2026-09-12.

---

## 0. End state

HookEcho should become a **professional U.S. severe-weather and mesoscale analysis workstation**, not merely a radar viewer.

The completed product should provide:

1. Near-real-time NEXRAD Level II display with progressive in-progress sweep updates, exact latency/provenance, and automatic feed failover.
2. Full Level II / Level III / TDWR / MRMS analysis with analyst-grade probing, derived products, 2D/3D cross sections, multi-moment 3D, and programmable user-defined products.
3. A metadata-driven MRMS browser covering the useful operational catalog rather than a short hard-coded subset.
4. Native GOES ABI ingest for CONUS/full disk/mesoscale sectors, including 1-minute mesoscale imagery, all ABI channels needed for operational analysis, GLM, and derived RGB recipes.
5. A generic model engine capable of HRRR, RAP, GFS, RRFS/REFS, GEFS, NBM and additional U.S.-relevant guidance without writing a custom renderer for every field.
6. Deterministic, ensemble, run-to-run, model-to-model, observed-vs-forecast and analysis-vs-forecast comparison workflows.
7. RTMA/URMA-quality surface analysis, improved observation blending, soundings, hodographs, objective analysis, and severe-weather diagnostics.
8. AWIPS-like multi-pane workspaces with linked time/cursor/map position and synchronized data probes.
9. Full GIS import/export and time-aware overlay support for professional emergency-management and research workflows.
10. Chase/navigation tooling that combines radar, warnings, storm motion, routing, offline data, and position sharing without claiming any route is “safe.”
11. Broadcast/headless/export tooling suitable for clean live graphics, automated imagery, video loops, and machine-readable data export.
12. Repeatable historical verification and backtesting for algorithms, alerts, warning performance, and derived products.
13. Reliable Windows/Linux/Android/Web operation with explicit data-source health, stale-data behavior, caching, resource budgets and reproducible tests.

### Definition of “top tier analyst tool”

A feature is not complete merely because it can be displayed. Analyst-grade features must expose:

- **source**
- **issue/run time**
- **valid time**
- **ingest time**
- **display latency / age**
- **units**
- **native grid / resolution**
- **interpolation / smoothing state**
- **quality flags where available**
- **derived-vs-observed-vs-forecast classification**
- **point sampling**
- **exportability** where practical

Every major layer must be usable in both a “quick visual” workflow and a “prove exactly what I am looking at” workflow.

---

# 1. Preserve what HookEcho already does well

Do **not** reimplement these from scratch unless refactoring is necessary:

- all six NEXRAD Level II base moments
- super-resolution polar rendering
- velocity dealiasing
- storm-relative velocity
- GRLevelX `.pal` support
- split panes / four panes
- multi-tilt views
- cross sections
- CAPPI
- current 3D volume view
- Level II archive back to 1991
- archived warnings and local storm reports on the timeline
- warning verification metrics
- SCIT cell tracks and storm table
- client-side tornado debris signature detection
- client-side rotation/couplet detection
- ProbSevere
- VIL / VIL density / echo tops derived from the Level II volume
- hail-size / severe-hail calculations already implemented
- MRMS reflectivity / QPE / MESH / rotation / azimuthal shear / precip type / FLASH products already implemented
- GLM support
- HRRR future reflectivity / forecast wind / smoke / rotation tracks
- RAP analysis and existing severe parameters
- Skew-T / hodograph / VAD / observed RAOB comparison
- effective-layer severe parameters
- SPC / WPC / NHC / aviation / winter layers already present
- METAR / NDBC / PWS layers
- chase GPS / offline chase packs / position sharing
- workspaces
- placefiles
- headless snapshots
- GIF / MP4 export
- OBS / streamer mode
- web app
- Android app
- storage/cache UI

The new architecture should make these capabilities easier to extend rather than replacing proven code.

---

# 2. Mandatory engineering rules for coding agents

These rules apply to every roadmap phase.

## 2.1 Do not grow `app.rs` further

`crates/hookecho/src/app.rs` is already extremely large. New substantial systems must not be implemented as more giant blocks in `app.rs`.

Before or during each feature:

- place acquisition/decoding/domain logic in `crates/wxdata`
- place rendering logic in `crates/hookecho/src/render/`, focused renderer modules, or shaders
- place persistent app state in dedicated structs/modules
- place UI panels under `crates/hookecho/src/ui/` or `crates/hookecho/src/app/`
- make `app.rs` orchestrate rather than own every algorithm

**Soft target:** reduce `app.rs` over time by moving cohesive systems out of it. Do not block feature work on a single massive refactor, but every major phase should leave it smaller or no larger.

## 2.2 No source-specific UI logic when a registry can solve it

Do not add 80 MRMS products as 80 booleans, 30 model fields as 30 UI branches, or GOES channels as hard-coded menu fragments.

Build reusable metadata registries.

Each gridded product should eventually describe itself with something equivalent to:

```rust
pub struct FieldDescriptor {
    pub id: FieldId,
    pub source: DataSource,
    pub family: FieldFamily,
    pub display_name: &'static str,
    pub short_name: &'static str,
    pub units: UnitSpec,
    pub value_kind: ValueKind,
    pub default_palette: PaletteId,
    pub native_resolution: ResolutionSpec,
    pub accumulation: Option<Duration>,
    pub vertical_level: Option<VerticalLevel>,
    pub supports_contours: bool,
    pub supports_vectors: bool,
    pub supports_sampling: bool,
    pub supports_difference: bool,
}
```

The exact names may differ, but the architectural principle is mandatory.

## 2.3 Universal time/provenance model

Add a common metadata object used by radar, satellite, MRMS, models and observations:

```rust
pub struct DataStamp {
    pub source_id: String,
    pub product_id: String,
    pub issue_time: Option<DateTime<Utc>>,
    pub run_time: Option<DateTime<Utc>>,
    pub valid_time: DateTime<Utc>,
    pub received_time: DateTime<Utc>,
    pub source_latency: Option<Duration>,
    pub is_forecast: bool,
    pub is_derived: bool,
    pub quality: QualitySummary,
}
```

Every layer legend/inspector should be able to answer “what exactly is this and how old is it?” without source-specific code.

## 2.4 Cancellable background work

All heavy fetch/decode/regrid/derived-product jobs must:

- run off the UI thread
- be cancellable when the user changes product/time/site
- ignore stale results using request generation IDs
- expose loading/error/stale status
- avoid duplicate identical downloads

On WASM, use the existing worker approach or additional Web Workers where necessary.

## 2.5 Source failures must degrade, not crash

Every remote feed must have:

- timeout
- retry with bounded exponential backoff
- stale cache fallback where meaningful
- source health state
- human-readable error
- no panics on malformed remote data

## 2.6 Tests are part of the feature

Every phase below includes acceptance tests. Do not mark a checkbox complete until its tests exist.

## 2.7 Preserve HookEcho’s privacy model

Do not add mandatory accounts, telemetry, analytics or a hosted HookEcho backend as a prerequisite for core weather data.

Optional user-configured relays/providers are allowed when required for specialized low-latency or collaboration workflows.

---

# 3. Phase A — Core product/data architecture

**Priority: P0. Required before broad model/MRMS/satellite expansion.**

## A1. Generic field/product registry

Create a common description layer for scalar grids, vectors, categorical grids and radar-derived fields.

### Implement

- [x] `FieldId` stable identifier ([evidence](us-analyst-implementation.md))
- [x] `DataSource` enum/ID ([evidence](us-analyst-implementation.md))
- [x] `FieldFamily`: radar / MRMS / satellite / model / analysis / observation-derived / user-defined
- [x] `ValueKind`: scalar / categorical / vector / probability / accumulation / mask
- [x] unit metadata and conversion ([evidence](us-analyst-implementation.md))
- [x] default palette and range
- [x] contour interval defaults
- [x] missing-data semantics
- [x] valid domain / bounds
- [x] native grid metadata
- [x] product search aliases
- [x] favorite/recent products
- [x] source/provenance inspector ([evidence](us-analyst-implementation.md))

### Integrate first

Migrate existing:

- MRMS fields from `crates/wxdata/src/mrms.rs`
- HRRR/RAP gridded fields
- global model fields used by `fielddiff.rs`

Do not migrate every layer at once. Prove the registry on those three families, then use it for all new work.

### Acceptance criteria

- adding a new MRMS scalar product requires a descriptor + fetch mapping, not a new menu implementation
- the layer browser can search by product name, source, unit and category
- legends are created from product metadata
- sampling uses one common API
- provenance UI works for migrated products

---

## A2. Unified timeline alignment engine

The current timeline is radar-centered. Convert it into a general valid-time coordinator.

### Add time policies

- `Exact`
- `Nearest`
- `NearestPast`
- `InterpolateLinear` where scientifically valid
- `HoldLast` for warnings/observations
- `ForecastLead`

### Implement

- [x] one selected analysis time shared across panes ([evidence](us-analyst-implementation.md))
- [x] per-layer time offsets visible in the UI
- [x] valid-time alignment for model differences
- [x] run-time alignment for run-to-run comparison
- [x] radar/satellite/MRMS nearest-frame synchronization ([evidence](us-analyst-implementation.md))
- [x] “lock all panes to valid time” toggle
- [x] “lock to source frame” option for exact radar analysis
- [x] explicit warning when sources differ by more than a configurable tolerance

### Acceptance criteria

Opening radar + GOES + MRMS + HRRR in four panes and scrubbing time keeps all panes at the nearest scientifically appropriate valid time while showing each layer’s exact source time.

---

## A3. Data cache abstraction

Native disk cache exists; browser persistence remains a roadmap concern.

### Implement

- [x] common cache interface for native and WASM ([evidence](us-analyst-implementation.md))
- [x] browser IndexedDB or OPFS persistence ([evidence](us-analyst-implementation.md))
- [x] cache namespaces by source/product/run
- [x] size quota per source family
- [x] LRU eviction
- [x] immutable object cache for archived frames
- [x] partial/range-response caching where useful for GRIB
- [x] checksum/content-length verification when available
- [x] storage statistics in existing Storage UI

### Acceptance criteria

Reloading the web app does not redownload unchanged radar/model/satellite data already cached locally, within configured quotas.

---

# 4. Phase B — Real-time NEXRAD / low-latency radar

**Priority: P0. This is the biggest operational-radar upgrade.**

Current HookEcho live chunks are already fast, but a top-tier radar workstation should show what has arrived **inside an in-progress sweep**, expose latency, and tolerate provider failures.

## B1. Radar provider abstraction

Create a provider trait around Level II live acquisition.

Conceptually:

```rust
trait Level2LiveProvider {
    async fn health(&self) -> ProviderHealth;
    async fn subscribe_site(&self, site: RadarSite, tx: Sender<RadialUpdate>);
    async fn latest_complete_volume(&self, site: RadarSite) -> Result<Volume>;
}
```

### Providers

- [x] current Unidata/AWS chunk source
- [x] completed-volume fallback from NOAA/AWS archive/current objects where applicable
- [ ] optional user-configured direct/LDM/NOAAPort-compatible relay provider

**Do not claim sub-10-second performance unless the active provider actually supplies data that quickly.** The UI must report measured latency rather than marketing a fixed number.

## B2. Progressive radial rendering

Instead of waiting for a sweep/volume boundary:

- [x] decode and publish radial blocks as they arrive
- [x] update GPU polar texture incrementally
- [x] preserve previous sweep underneath not-yet-updated azimuths
- [x] visually distinguish “new scan”, “old scan” and “not yet received” when analyst scan-progress mode is enabled
- [x] expose current elevation, VCP, sweep number and scan progress
- [x] show age since radar timestamp and age since local receipt separately
- [x] keep animation smooth while updates stream

Repeated-cut coverage is checked by `live::tests::repeated_low_cut_with_new_elevation_number_retains_older_azimuths` and `repeated_split_cut_does_not_borrow_radials_from_another_moment`.

## B3. Latency dashboard

Add a compact source/latency diagnostic:

- beam/radial timestamp where available
- current wall-clock difference
- provider ingest delay
- decode/render delay
- latest complete volume age
- dropped/retried chunks
- provider failover state

## B4. Radar metadata inspector

For a sampled gate expose:

- radar site
- VCP
- elevation angle
- azimuth
- slant range
- ground range
- beam center height using current 4/3-earth model
- gate spacing
- raw product value
- dealiased value where relevant
- Nyquist velocity
- range-folded/missing state where available
- sweep timestamp

## B5. VCP / SAILS / MESO-SAILS awareness

- [x] parse/display current VCP details
- [x] identify repeated low-level cuts
- [x] show scan strategy in analyst panel
- [ ] make timeline order reflect actual sweep chronology
- [x] allow “follow newest 0.5° cut” mode independent of full-volume completion

The optional per-pane control is in the Analyst inspector. `view::tests::low_cut_follow_is_live_only_and_leaves_missing_tilts_alone` covers live pinning, archive browsing, and sites without a 0.5° cut; the live path applies it after each partial scan update.

The inspector lists cuts by their radar collection times, including repeated angles, and the live scrubber clock/age uses the newest collected cut. Playback still uses volume slots; cut-level scrubbing remains open.

## B6. Feed failover

- [ ] provider priority list
- [ ] health probes
- [x] automatic failover after bounded failure criteria
- [ ] manual provider override in advanced settings
- [x] no hidden mixing of timestamps—provider changes are recorded in provenance

### Acceptance tests

- synthetic chunk stream arrives out of order and produces correct final sweep
- missing radial block does not corrupt adjacent azimuths
- provider failure mid-scan falls back cleanly
- measured latency is shown correctly
- completed volume matches the same archived Level II volume within decode tolerance

---

# 5. Phase C — Advanced radar analysis engine

**Priority: P0/P1. This is what moves HookEcho from viewer to analyst workstation.**

## C1. User-defined radar product engine

Implement a safe expression/DSL system inspired by the flexibility of GR2Analyst user-defined products, but designed around HookEcho’s Rust/WGPU architecture.

### First version capabilities

Inputs:

- REF
- VEL
- SW
- ZDR
- CC
- KDP
- gate altitude
- range
- azimuth
- elevation
- freezing level / -10C / -20C environmental heights when available

Functions:

- min / max / mean
- clamp
- conditional masks
- threshold
- vertical max/min
- layer max/min/mean
- first/last height crossing
- count gates meeting condition
- arithmetic

Example conceptual expressions:

```text
max_vertical(REF where REF >= 40)
min_vertical(CC where REF >= 35)
max_layer(ZDR, freezing_level + 2km, freezing_level + 6km)
max_layer(KDP, minus10c_height, minus20c_height)
```

### Safety/implementation constraints

Do **not** execute arbitrary native code.

Preferred implementation order:

1. parsed AST evaluated on CPU for correctness
2. typed expression validation
3. optional AST-to-WGSL generation for parallel products
4. deterministic resource limits

### Product definition format

Use TOML/JSON/YAML-like portable definitions containing:

- name
- units
- input moments
- expression
- default palette
- min/max
- missing value
- optional environmental requirements

### Acceptance criteria

- invalid formulas cannot crash the app
- same formula produces matching CPU/GPU results within tolerance
- product can be saved, synced and exported
- user-defined product can be rendered in a normal pane and sampled

---

## C2. Maximum-value trails / temporal extrema

Add analyst trails for values over a moving time window.

Use cases:

- azimuthal shear maximum
- ZDR column maximum
- reflectivity core path
- MESH / hail-core path
- CC minimum path
- user-defined products

### Controls

- window: 15/30/60/120 minutes/custom
- decay visualization
- threshold
- min or max mode
- reset at selected archive time
- export raster/vector trail

### Acceptance criteria

Historic supercell replay produces a stable rotation/hail trail that can be independently recomputed from cached frames.

---

## C3. Beam geometry / blockage / coverage analysis

HookEcho already models beam height. Extend it into a full analysis layer.

- [x] beam center and approximate beamwidth top/bottom
- [x] height readout at cursor
- [x] lowest usable beam map
- [x] terrain blockage estimate using existing elevation infrastructure
- [x] radar coverage comparison between neighboring sites
- [x] optional beam-rise overlay in cross-section and 3D
- [x] warn when a sampled feature is below/above sampled beam coverage

Do not imply perfect propagation; clearly label 4/3-earth assumptions.

---

## C4. Multi-moment correlation tools

Add linked probes and scatterplots:

- REF vs ZDR
- REF vs CC
- ZDR vs KDP
- VEL vs CC
- selectable polygon/box area statistics
- histogram for selected region
- vertical profile at point
- time series at fixed lat/lon

Export CSV for all statistics.

---

## C5. Algorithm laboratory

Turn current TDS/couplet/cell scoring into an inspectable analyst environment.

For every automatic detection:

- algorithm version
- thresholds used
- contributing gates/cells
- confidence/score
- reason codes
- timeline of score changes

Allow historical backtest against:

- SPC LSR tornado/hail/wind reports
- NWS DAT damage surveys
- warning polygons

Do not present heuristic detections as official warnings.

---

# 6. Phase D — MRMS full catalog

**Priority: P0.**

The current `mrms.rs` contains a valuable but hand-selected subset. Replace the hard-coded-growth model with a metadata-driven MRMS catalog.

## D1. MRMS product catalog

Create `wxdata::mrms::catalog` (or equivalent) with descriptors.

Target operational groups:

### Radar / reflectivity

- composite reflectivity variants
- low-level reflectivity
- echo tops
- vertically integrated products
- layer-height products

### Severe

- MESH and MESH swaths
- POSH / hail probabilities where available
- azimuthal shear low/mid layers
- rotation tracks for all published windows
- severe probability products where published

### Precipitation

- precip rate
- radar-only QPE
- multi-sensor QPE
- 1/3/6/12/24-hour accumulations as available
- gauge-corrected variants
- precipitation flag/type

### Hydrology / flash flood

- FLASH ARI products
- QPE-to-ARI exceedance fields
- streamflow-related products where publicly available and operational

### Lightning

- all public MRMS lightning density windows used operationally

### Winter

- MRMS snow/precipitation-type products when published in the operational bucket

### Rules

Do not blindly list a product unless a feed contract test confirms it exists.

## D2. Generic MRMS fetch/decode path

One code path should support any catalog scalar grid:

- path template
- latest-file discovery
- gzip handling
- GRIB decode
- product-specific scale/missing rules
- max texture dimension handling
- correct interpolation method by field type

Categorical products must never use bilinear interpolation.

## D3. MRMS browser UI

Add:

- search
- category
- favorites
- recent
- accumulation selector
- valid time
- native resolution
- source age
- point sample
- animation

## D4. MRMS 3D/vertical products

Where source data has vertical layers or layer heights:

- vertical profile inspector
- layer stack
- optional 3D surface/volume representation

Do not fabricate 3D from a 2D surface product.

### Acceptance criteria

- new scalar MRMS product can be added through catalog metadata with minimal/no new UI code
- at least the major WeatherFront-class MRMS groups are covered
- categorical fields use nearest-neighbor
- all products show exact valid time and units

---

# 7. Phase E — Native GOES ABI workstation

**Priority: P0.**

Current GIBS imagery is useful but insufficient for an analyst-grade satellite workstation. Add native NOAA GOES-R ABI products.

## E1. Native ingest

Use NOAA public GOES S3 data.

Support:

- GOES-East
- GOES-West
- ABI L2 Cloud and Moisture Imagery (CMIP)
- CONUS sector
- mesoscale sectors 1 and 2
- full disk where useful

For U.S. scope, prioritize CONUS + mesoscale before full disk.

## E2. ABI channel support

Implement analyst channels at native/reasonable resolution:

- C01 blue
- C02 red
- C03 veggie
- C05 snow/ice
- C07 shortwave IR
- C08 upper-level water vapor
- C09 mid-level water vapor
- C10 lower-level water vapor
- C13 clean IR
- C14 longwave IR
- C15 dirty IR
- other channels necessary for RGB recipes

Preserve DQF/quality masks where practical.

## E3. Projection

Implement correct GOES fixed-grid geostationary projection.

Prefer shader projection rather than baking every frame to Web Mercator when possible.

Test known landmarks against NOAA imagery.

## E4. RGB recipe engine

Create reusable recipe definitions rather than hard-code each RGB.

Target:

- GeoColor / true-color-style composite
- Day Cloud Phase
- Day Convection
- Air Mass
- Dust
- Fire Temperature
- Night Microphysics
- Sandwich product

Recipe metadata should specify channel inputs, transforms, gamma/ranges and output meaning.

## E5. Rapid-scan handling

- discover active mesoscale sector footprints
- 1-minute frame timeline
- dynamically follow sector movement
- show sector boundary
- graceful switch when the event exits the meso sector

## E6. Satellite analysis tools

- brightness-temperature sample
- channel difference products
- cold-cloud-top threshold overlay
- cooling-rate/time-change product
- GLM overlay synchronized to frame
- radar + satellite dual/quad pane presets

## E7. Offline satellite chase packs

Allow selected time/range/sector frames to be downloaded into chase packs subject to storage budget.

### Acceptance criteria

- live 1-minute mesoscale frames animate correctly when available
- clean IR and water-vapor values can be sampled numerically
- radar, GLM and satellite align by valid time
- quality/missing pixels are distinct from cold/low values

---

# 8. Phase F — Generic U.S. model and ensemble workstation

**Priority: P0.**

Do not build each model as a separate feature. Build a general GRIB model engine and add model definitions.

## F1. Model abstraction

A model definition must include:

- model ID
- provider/base URL
- run cadence
- forecast-hour schedule
- grid/projection
- variable mappings
- vertical-coordinate mappings
- byte-range/index strategy
- expected latency
- domain
- ensemble membership if applicable

Create common APIs for:

- list runs
- list forecast hours
- fetch field
- fetch vertical profile
- regrid/sample
- compare

## F2. Model priority list

### Tier 1

- [x] HRRR — migrate existing functionality to generic engine where sensible ([evidence](us-analyst-implementation.md))
- [x] RAP — migrate existing analysis/profile functionality ([evidence](us-analyst-implementation.md))
- [x] GFS — expand beyond current comparison fields ([evidence](us-analyst-implementation.md))
- [x] RRFSv1 deterministic ([evidence](us-analyst-implementation.md))
- [x] REFS / RRFS ensemble members ([evidence](us-analyst-implementation.md))
- [x] GEFS ([evidence](us-analyst-implementation.md))
- [x] NBM ([evidence](us-analyst-implementation.md))

### RRFS timing note

As of 2026-09-12, NOAA’s current published implementation schedule lists RRFS/REFS v1 operational implementation for **2026-10-06**. Build the adapter against the available NOAA/AWS parallel feed now, but do not assume final operational naming/availability until contract tests pass after implementation.

### Tier 2

- [x] ECMWF open IFS fields useful over the U.S. if current licensing/access remains compatible ([evidence](us-analyst-implementation.md))
- [ ] ECMWF ensemble products only where openly and legally retrievable
- [ ] NOAA-accessible AI guidance such as GraphCast products where stable public feeds exist
- [ ] experimental guidance behind an explicit EXPERIMENTAL label

Do not depend on fragile scraped images when machine-readable grids exist.

## F3. Generic field set

Support by metadata rather than bespoke UI.

### Surface

- 2 m temperature
- 2 m dewpoint
- RH
- 10 m wind/gust
- MSLP
- visibility
- precipitation type
- accumulated QPF
- snowfall
- freezing rain / ice where available
- simulated reflectivity
- updraft helicity / rotation proxies
- smoke/aerosol where available

### Pressure levels

- height
- temperature
- dewpoint/RH
- wind
- vertical velocity
- vorticity

At minimum: 1000/925/850/700/500/300/250/200 hPa where the model provides them.

### Severe diagnostics

- SBCAPE
- MLCAPE
- MUCAPE
- CIN
- LCL/LFC/EL
- 0–1 / 0–3 SRH
- bulk shear layers
- STP
- SCP
- EHI
- effective-layer diagnostics

Where source grids do not publish the diagnostic, compute it from the vertical profile using one shared meteorological calculation library rather than different formulas per model.

## F4. Model display modes

Every appropriate field should support:

- shaded fill
- contours
- barbs
- arrows
- wind particles
- streamlines where feasible
- point sample
- time series

## F5. Run-to-run comparison

Example:

- current HRRR run minus previous HRRR run at same valid time

Support:

- scalar difference
- absolute difference
- percentage difference where meaningful
- threshold highlighting
- synchronized side-by-side panes

## F6. Model-to-model comparison

Expand existing `fielddiff.rs` into a general comparison system.

Modes:

- A - B difference
- side by side
- swipe divider
- blink A/B
- disagreement mask

## F7. Ensemble workstation

This is required for top-tier analysis.

For GEFS/REFS and any supported ensemble:

- individual member view
- ensemble mean
- ensemble spread / standard deviation
- min/max
- percentile fields
- probability of threshold exceedance
- neighborhood probability when scientifically appropriate
- member postage-stamp grid
- spaghetti contours
- point plume/time series
- ensemble sounding overlay

Probability examples:

- CAPE > threshold
- wind gust > threshold
- QPF > threshold
- snow > threshold
- UH > threshold for convection-allowing ensembles

## F8. Point sounding overhaul

Point sounding should support:

- any supported deterministic model
- ensemble member soundings
- ensemble envelope
- observed RAOB overlay
- previous model run overlay
- parcel selection
- Bunkers vectors
- effective inflow layer
- freezing/-10/-20/-30 C heights
- DCAPE
- lapse rates
- PWAT
- hodograph layer coloring
- storm-motion user override
- downloadable CSV

### Acceptance criteria

Adding a new model with an already-supported GRIB/projection format should primarily require a model definition + field mappings, not a new renderer.

---

# 9. Phase G — RTMA/URMA and objective surface analysis

**Priority: P1.**

## G1. RTMA/URMA

Add U.S. Real-Time Mesoscale Analysis / UnRestricted Mesoscale Analysis fields where publicly available:

- 2 m temperature
- dewpoint
- 10 m wind
- wind gust where available
- pressure
- visibility
- precip analysis fields as appropriate

Expose analysis age and distinguish RTMA real-time analysis from URMA retrospective analysis.

## G2. Observation + analysis blend

Finish the existing roadmap item about blending surface observations into effective-layer analysis.

Use:

- METAR
- NDBC
- supported PWS feeds
- RTMA/URMA
- RAP/HRRR profiles aloft

The purpose is not to pretend the result is official SPC mesoanalysis. Label it clearly as **HookEcho objective analysis**.

## G3. Surface objective analysis tools

- station residuals vs analysis
- contour generation
- dewpoint gradient
- theta-e
- moisture convergence if scientifically valid from available gridded winds/moisture
- temperature advection
- frontogenesis diagnostics later

## G4. Time-series station/analysis comparison

At a selected point/station show:

- observed
- RTMA
- HRRR analysis
- forecast model traces

This is useful for model bias and boundary evolution.

---

# 10. Phase H — Advanced 3D and cross-section workstation

**Priority: P1.**

Current `wxdata::volume3d` resamples reflectivity into a Cartesian 3D grid and `render3d.rs` displays it. Build on this rather than replacing it blindly.

## H1. Multi-moment 3D

Support 3D for:

- reflectivity
- velocity/SRV where scientifically interpretable
- ZDR
- CC
- KDP
- user-defined products

## H2. Transfer-function editor

Add analyst controls:

- opacity vs value curve
- color transfer function
- threshold clipping
- value window
- vertical exaggeration
- quality mask

Save presets per product.

## H3. Isosurfaces

Implement GPU/CPU isosurface generation, preferably marching cubes or equivalent.

Examples:

- 40/50/60 dBZ surfaces
- low-CC debris region
- ZDR column threshold

Controls:

- threshold
- transparency
- lighting
- smoothing toggle

Never smooth the source values silently; smoothing must be explicit display processing.

## H4. Movable clipping and slicing planes

- arbitrary vertical plane
- horizontal CAPPI plane
- slab thickness
- clip box
- storm-centered clip
- cross-section line visible in map pane

## H5. Beam visualization in 3D

Show:

- individual elevation cones
- beam centerline
- approximate beamwidth
- radar location/elevation
- terrain surface option

## H6. 3D overlay fusion

After radar 3D is mature, add optional:

- MRMS layer-height surfaces
- satellite cloud-top-height surface when a trustworthy source exists
- model isosurfaces for selected scalar fields

Keep observed radar, analyzed MRMS and forecast model geometry visually distinct.

## H7. 3D performance targets

- desktop target: interactive 60 fps on a representative mid-range GPU for default volume resolution
- mobile/web target: adaptive resolution with explicit quality setting
- avoid rebuilding volume grid when only camera changes
- cache 3D textures per volume/product

---

# 11. Phase I — Professional GIS engine

**Priority: P1.**

Placefiles are not enough for emergency-management, research and broadcast users.

## I1. Import formats

Implement in this order:

1. [ ] GeoJSON
2. [ ] ESRI Shapefile (`.shp/.shx/.dbf`, optional `.prj`)
3. [ ] KML
4. [ ] KMZ
5. [ ] GeoPackage if a cross-platform Rust path is practical

## I2. Projection handling

- parse CRS from source metadata
- transform to WGS84/Web Mercator display coordinates
- support common U.S. EPSG projections
- reject unknown projections with a useful error instead of silently misplacing geometry

## I3. Geometry types

- point
- multipoint
- line
- multiline
- polygon
- multipolygon

## I4. Styling

- stroke color/width
- fill/opacity
- labels from chosen attribute
- symbol by category
- graduated color by numeric attribute
- visibility by zoom
- z-order

## I5. Time-aware GIS

Allow a user to map attributes to:

- valid start
- valid end

Then hide/show features with the HookEcho timeline.

## I6. GIS export

Export:

- drawn annotations to GeoJSON
- storm tracks to GeoJSON
- selected warning geometry
- sampled/threshold contours
- route geometry

### Acceptance criteria

A county-level shapefile in a non-WGS84 but declared projection renders in the correct U.S. location and can be styled by attribute.

---

# 12. Phase J — AWIPS-style analyst workspace

**Priority: P1.**

HookEcho already has saved workspaces and up to four panes. Extend this into a serious analysis layout system.

## J1. Layouts

Support:

- 1 pane
- 2 horizontal/vertical
- 3 pane
- 4 pane
- 6 pane
- 9 pane on desktop/web where practical
- AWIPS-style asymmetric layouts

Android may use fewer panes based on screen size.

## J2. Link groups

Each pane should independently join link groups for:

- camera/location
- zoom
- time
- cursor/crosshair
- radar site
- storm selection

This enables, for example, four products locked in location/time but not product.

## J3. Synchronized crosshair/probe

Moving cursor in one pane should optionally show corresponding point in linked panes and a compact table:

| Pane | Source | Product | Time | Value |
|---|---|---|---|---|

## J4. Compare modes

- swipe
- blink
- difference
- transparent overlay
- side-by-side

## J5. Analyst presets

Ship presets such as:

### Tornado analysis

- 0.5 REF
- 0.5 SRV
- 0.5 CC
- 0.5 ZDR
- ProbSevere/storm table visible

### Hail analysis

- REF
- ZDR
- CC/KDP
- MESH
- sounding panel

### Mesoscale analysis

- surface theta-e/dewpoint
- CAPE
- SRH/shear
- satellite

### Forecast comparison

- HRRR
- RRFS
- ensemble probability
- observed/MRMS

## J6. Keyboard-first workflows

Add shortcuts for:

- product next/previous
- tilt next/previous
- previous/next frame
- live
- pane focus
- link/unlink
- sample tool
- cross section
- sounding
- 3D

Every shortcut must appear in command palette/help.

---

# 13. Phase K — Forecast verification and research/backtesting

**Priority: P1/P2.**

HookEcho already has warning verification. Extend the philosophy to model and algorithm verification.

## K1. Model-vs-observation verification

At a point or region compare forecasts against:

- METAR
- RAOB
- RTMA/URMA
- MRMS precip/reflectivity where scientifically appropriate

Metrics:

- bias
- MAE
- RMSE
- timing error
- categorical hit/miss/false alarm for thresholds

## K2. Radar algorithm verification

Backtest:

- TDS detection
- rotation/couplet detection
- hail diagnostics
- user-defined products

Against:

- LSRs
- DAT surveys
- SPC tornado database where applicable

## K3. Case-study package

Allow user to create a portable case manifest containing:

- event name
- time range
- radar sites
- enabled products
- annotations
- bookmarks
- optional cached public-data files within size limits

## K4. Analyst notebook/export

Provide a generated report/export directory containing:

- screenshots
- CSV probes
- GeoJSON annotations
- metadata/provenance JSON
- settings/product definitions

No need to build a word processor; make HookEcho outputs reproducible in external analysis tools.

---

# 14. Phase L — U.S. route/chase analysis

**Priority: P2 for analyst tool; P1 for chase users.**

HookEcho already has GPS, chase HUD, offline basemap packs and position sharing. Build weather-aware route analysis on top.

## L1. Route engine abstraction

Support pluggable route sources:

- user-configured OSRM/Valhalla endpoint initially
- optional public routing provider only if terms permit
- later offline routing graph included in a chase pack if size/performance is acceptable

Never hard-code dependency on a paid provider.

## L2. Route display

- start / waypoint / destination
- live GPS progress
- ETA
- distance
- alternate routes when provider supplies them

## L3. Weather exposure analysis

Sample along the route against:

- warning polygons
- current radar
- MRMS precip/MESH/FLASH
- lightning
- storm-motion cones
- forecast radar/model fields

Output should say things like:

- “route intersects active tornado warning polygon in 18 mi”
- “forecast path intersects heavy reflectivity between 21:10–21:25Z”

Do **not** label a route safe or guarantee avoidance.

## L4. Storm intercept geometry

For a selected tracked storm and route:

- closest approach
- relative bearing
- projected intersection time
- storm ETA vs vehicle ETA
- escape-direction visualization already present should integrate with route

## L5. Offline chase pack v2

Pack selection UI should optionally include:

- basemap tiles
- radar-site metadata
- recent radar frames
- selected GOES frames
- selected MRMS frames
- routing graph when implemented
- saved placefiles/GIS layers

Show estimated pack size before download.

---

# 15. Phase M — Broadcast, headless and interoperability

**Priority: P1/P2.**

## M1. Broadcast output workspace

Build on streamer/OBS mode.

Presets:

- 1920x1080
- 2560x1440
- 3840x2160
- portrait/social
- transparent-background overlay where renderer supports it

Controls:

- safe margins
- legend visibility
- clock/source stamp
- warning crawl optional
- logo/branding slot optional

## M2. Deterministic capture

- fixed-resolution offscreen rendering independent of current window size
- PNG/JPEG/WebP still
- MP4/GIF existing export upgraded to use exact timeline timestamps
- variable/fixed frame interval option
- metadata sidecar JSON

## M3. Automated output

Extend headless mode:

- render named workspace
- render selected time/range/site/product
- scheduled repeating snapshot
- update only when source valid time changes
- optional atomic file replace for web overlays

## M4. Local API

Extend/standardize the existing local serve capabilities with a documented API:

- current view state
- source health
- current warnings
- sampled point
- available products
- latest frame timestamps
- snapshot endpoint

Optional WebSocket/SSE for state/frame-change events.

Keep it localhost by default; explicit config required to bind externally.

## M5. Scientific export

Add where practical:

- GeoTIFF for georeferenced scalar grids
- NetCDF for gridded fields
- CF/Radial-compatible export for selected radar data if feasible
- CSV for probes/profiles/tables
- GeoJSON for vectors/tracks/polygons
- JSON metadata/provenance

---

# 16. Phase N — Data reliability and source observability

**Priority: P0 across all phases.**

Top-tier operational software needs to make feed quality visible.

## N1. Data Source Health panel

For every active source:

- provider
- endpoint family
- last successful request
- latest valid data time
- age
- expected cadence
- rolling success/failure count
- current backoff
- cache state
- fallback provider

Status states:

- Live
- Delayed
- Stale
- Failed
- Cached
- Experimental

## N2. Stale-data policy

Define per source family.

Examples:

- radar: prominently stale after expected scan cadence threshold
- METAR: normal hourly cadence, do not mark stale after 10 minutes
- model: show run age rather than simplistic stale flag

## N3. Provider contract tests

Create network tests that run on schedule, not every PR, for public feeds:

- latest Level II chunk listing
- latest MRMS file
- GOES ABI object listing
- HRRR/RAP/GFS byte-range/index access
- RRFS/REFS feed
- RTMA/URMA feed

Alert CI maintainers when schemas/paths change.

## N4. Local diagnostics bundle

User can export a diagnostics text/JSON file containing:

- app version
- platform
- renderer/backend
- source health
- recent errors
- cache sizes
- performance counters

No location history, API keys or private tokens in the bundle.

---

# 17. Phase O — Performance and memory engineering

**Priority: continuous.**

## O1. Establish budgets

Create benchmark targets for representative hardware.

### Desktop

- smooth pan/zoom at 60 fps for normal radar + overlays
- no UI-thread decode stalls > 16 ms for steady-state streaming
- 3D interactive target 60 fps default quality, degrade gracefully

### Web

- startup bundle budget remains enforced
- first useful radar frame benchmark
- Web Worker decode benchmark
- IndexedDB/OPFS cache benchmark

### Android

- memory budget by device class
- adaptive GPU texture sizes
- battery-aware animation/particle quality

## O2. GPU grid tiling

Some MRMS/model products exceed common texture dimensions. Move from whole-grid assumptions toward tiled/virtualized textures where needed.

Requirements:

- visible-region upload
- correct sampling at tile boundaries
- no max-pooling for fields where extrema preservation would distort meaning
- field-specific downsample operator: nearest / mean / max / vector-safe

## O3. Decode and regrid caching

Cache separately:

1. compressed bytes
2. decoded native grid
3. regridded display representation
4. GPU texture

Invalidate only the layers actually affected by a settings change.

## O4. Profiling

Expand existing profiling/perf infrastructure:

- fetch
- decompression
- decode
- regrid
- derived calculation
- upload
- draw

Display optional developer overlay.

---

# 18. Phase P — Plugin/extension architecture

**Priority: P2, but design hooks earlier.**

HookEcho already has an external-process plugin runner. Turn it into a documented analyst extension path without compromising reliability.

## P1. Stable plugin manifest

Plugin declares:

- name/version
- API version
- capabilities
- input products
- output products
- config schema

## P2. Safe product plugin path

Allow plugin to consume exported grid/radar samples and return:

- scalar grid
- vector features
- annotations
- table

Prefer an IPC protocol or sandboxed WASM over loading arbitrary dynamic libraries into the process.

## P3. Python interoperability

Do not embed Python as a hard dependency.

Instead provide:

- JSON/NDJSON IPC
- NetCDF/GeoTIFF/CSV exports
- documented local API

This lets researchers use Python/MetPy/Py-ART externally while HookEcho remains Rust-native.

---

# 19. Phase Q — UI/platform polish

**Priority: P1/P2.**

U.S.-scope does not mean desktop-only.

## Q1. Android tablet layout

Current phone chrome should not simply stretch.

- two-column controls on large screens
- persistent layer/product panel option
- multi-pane optimized touch targets
- keyboard/mouse support on tablets
- drag/drop pane layout where practical

## Q2. Desktop analyst density

Add compact mode:

- denser product tables
- dockable optional analyst panels while preserving current full-map default
- high-information status footer option

## Q3. Accessibility

Preserve current accesskit/high-contrast work and ensure new controls have:

- semantic names
- keyboard access
- non-color-only state indication
- scalable text

## Q4. macOS

Keep experimental support healthy if CI permits, but it is not allowed to delay core U.S. analyst phases.

## Q5. iOS

Still out of scope for this roadmap unless project constraints change. Do not divert core analyst engineering into iOS before P0/P1 analysis goals are complete.

---

# 20. Additional U.S. situational-awareness layers

**Priority: P2 after the analysis core.**

Add only when reliable machine-readable sources exist.

Candidates:

- CPC temperature/precipitation outlooks
- CPC drought/outlook products
- additional WPC precipitation discussions/products
- NDFD/NBM forecast grids
- WFO/CWA boundaries and office information
- county/parish/zone boundary inspector
- TFRs if current aviation layer does not already cover required operational detail
- river/flood enhancements from NWPS
- public power-outage datasets where licensing/feed stability permits
- U.S. road/weather feeds through provider adapters, not 50 one-off UI implementations

Each must use the generic product/provenance system.

---

# 21. Top-tier “analyst extras” beyond competitor parity

These are not required for initial parity, but are high-value differentiators.

## R1. Multi-radar storm volume fusion

Research-grade feature.

For storms seen by overlapping WSR-88Ds:

- align volumes by valid time
- transform samples into common Cartesian volume
- retain contributing-radar provenance
- blend reflectivity using distance/beam-height-aware weighting

Do **not** create a “velocity mosaic” by naïvely combining radial velocities. If multi-Doppler wind retrieval is attempted, it must use proper geometry and be labeled experimental/research.

## R2. Dual-Doppler wind synthesis — experimental

Long-term research item only.

Requirements before implementation:

- overlapping radars
- synchronized time tolerance
- quality control
- solved horizontal wind components from radial velocities
- uncertainty/residual field
- clear invalid-geometry masking

This must never be implemented as simple pixel averaging.

## R3. Object-based storm history

Create persistent storm objects that combine:

- SCIT/ProbSevere objects
- radar-derived rotation/TDS/hail metrics
- MRMS severe fields
- warnings
- LSRs
- DAT surveys

Expose a storm history timeline and exportable JSON/CSV.

## R4. Feature tracking

Track analyst-selected radar features through time using optical flow/object matching:

- mesocyclone marker
- hail core
- reflectivity core
- bounded weak echo region candidate
- user-selected contour

Always show confidence and allow manual correction.

## R5. Uncertainty everywhere

Where products support it, visualize:

- ensemble spread
- timing spread
- radar beam sampling limitations
- source age
- quality flags

A top-tier analyst workstation should show uncertainty instead of hiding it.

---

# 22. Competitive parity checklist — U.S. only

This is the explicit “what are we still missing?” list for agents.

## WeatherFront-class gaps

- [x] native full-resolution GOES ABI
- [x] 1-minute mesoscale satellite
- [x] broad RGB/channel suite
- [x] generic 80+-class MRMS catalog coverage
- [x] RRFS/REFS
- [x] GEFS / ensemble probabilities
- [x] NBM ([evidence](us-analyst-implementation.md))
- [x] RTMA/URMA
- [x] richer model/run comparison
- [x] route planning
- [x] AWIPS-style pane layouts
- [ ] WFO boundary/contact tooling

## RadarOmega-class gaps

- [x] 3D for more than radar reflectivity
- [ ] 3D MRMS/model-derived surfaces where scientifically valid
- [x] broader model catalog
- [ ] long high-frame-count satellite/model/MRMS loops efficiently cached
- [ ] broader supplemental U.S. outlook/situational layers

## WeatherWise-class gaps

- [x] progressive in-progress sweep display
- [x] measured ultra-low-latency pipeline where provider permits
- [x] explicit beam-rise visualization
- [ ] more polished 3D cross-section workflow
- [x] impact/analysis report workflow

## WSV3-class gaps

- [x] in-progress LiveScan-style rendering
- [x] precise delay indicator
- [x] scan-age visualization
- [x] Shapefile GIS import
- [x] stronger broadcast output/capture workflows
- [ ] multi-provider operational redundancy

## GR2Analyst-class gaps

- [x] user-defined radar product system
- [x] maximum/minimum value trails
- [x] mature transfer-function 3D
- [x] isosurfaces
- [x] movable slicing planes / clip slabs
- [x] deeper radar metadata/quality inspection

## RadarScope-class operational gaps

- [ ] broader provider failover/redundancy
- [x] more source-health transparency
- [ ] tighter Spotter Network reporting integration if public/authenticated API terms permit
- [x] mature tablet experience

---

# 23. Recommended implementation sequence

Do not attempt all phases in parallel. Use this dependency order.

## Milestone 1 — Foundation

1. Phase A1 generic field registry
2. Phase A2 time/provenance model
3. Phase A3 persistent web cache
4. source health primitives from Phase N
5. begin shrinking `app.rs`

**Release goal:** no major visible feature required; architecture release.

## Milestone 2 — Operational radar

1. Level II provider abstraction
2. progressive radial rendering
3. latency/scan progress UI
4. VCP/SAILS awareness
5. provider failover
6. radar gate inspector

**Release goal:** HookEcho becomes a genuinely low-latency operational radar client.

## Milestone 3 — Satellite + MRMS breadth

Run in parallel after registry is stable:

- native GOES ABI
- generic MRMS catalog

**Release goal:** radar/MRMS/satellite can be compared on one synchronized timeline.

## Milestone 4 — Model workstation

1. generic model engine
2. HRRR/RAP migration
3. RRFS/REFS adapter
4. GFS expansion
5. GEFS
6. NBM
7. ensemble tools
8. run/model comparison

**Release goal:** no need to leave HookEcho for routine deterministic/ensemble mesoscale comparison.

## Milestone 5 — Analyst radar depth

1. user-defined product DSL
2. max-value trails
3. beam/blockage analysis
4. multi-moment stats/scatterplots
5. algorithm laboratory/backtesting

**Release goal:** compete directly with dedicated radar-analysis software rather than only weather apps.

## Milestone 6 — 3D + GIS + workspace

1. advanced 3D
2. GIS import/export
3. AWIPS layouts and linked probes
4. analyst presets

**Release goal:** professional workstation workflow.

## Milestone 7 — Verification / chase / broadcast

1. RTMA/URMA and objective analysis
2. model verification
3. route exposure tools
4. broadcast workspace
5. scientific export/local API

**Release goal:** field, research, EOC and broadcast usability.

## Milestone 8 — Research differentiators

- multi-radar volume fusion
- experimental dual-Doppler
- persistent storm objects
- advanced feature tracking

---

# 24. Suggested module/file map

This is guidance, not a rigid requirement. Reuse existing modules where they already own the concept.

## `crates/wxdata`

Suggested additions/refactors:

```text
src/
  field/
    mod.rs
    descriptor.rs
    units.rs
    grid.rs
    provenance.rs
  model/
    mod.rs
    catalog.rs
    grib.rs
    hrrr.rs
    rap.rs
    gfs.rs
    rrfs.rs
    gefs.rs
    nbm.rs
  satellite/
    mod.rs
    goes.rs
    abi.rs
    projection.rs
    rgb.rs
  mrms/
    mod.rs
    catalog.rs
    fetch.rs
  radar_analysis/
    mod.rs
    expression.rs
    user_product.rs
    trails.rs
    beam.rs
  analysis/
    rtma.rs
    urma.rs
    objective.rs
```

Do not move everything just to match this tree. Migrate when touching the relevant subsystem.

## `crates/hookecho`

Suggested ownership:

```text
src/
  app/
    data_controller.rs
    pane_state.rs
    layer_state.rs
    task_state.rs
  ui/
    product_browser.rs
    source_health.rs
    data_inspector.rs
    model_browser.rs
    ensemble.rs
    satellite.rs
    gis.rs
    route.rs
    analyst_workspace.rs
  render/
    field.rs
    satellite.rs
    gis.rs
    comparison.rs
    volume3d.rs
```

Existing files that should be extended/refactored rather than duplicated:

- `timeline.rs`
- `workspace.rs`
- `fielddiff.rs`
- `render3d.rs`
- `headless.rs`
- `serve.rs`
- `webcache.rs`
- `plugins.rs`
- `perf.rs`
- `profiling.rs`
- `elevation.rs`
- `chase.rs`
- `gps.rs`

---

# 25. Test strategy

## Unit tests

Required for:

- product metadata validation
- unit conversion
- time alignment
- GRIB field mapping
- GOES projection
- RGB transforms
- MRMS path mapping
- user-product parser/type checker
- beam geometry
- ensemble statistics
- GIS CRS transforms

## Golden-data tests

Maintain small legally redistributable or generated fixtures.

Suggested historical cases:

- 2013-05-20 KTLX Moore tornado
- 2013-05-31 KTLX El Reno
- 2011-04-27 KBMX severe convection
- 2021-12-11 Mayfield
- 2022-09-28 Hurricane Ian landfall

Use fixtures to test algorithms, not just screenshots.

## Screenshot regression

Add deterministic captures for:

- radar product
- cross section
- 3D
- GOES RGB
- MRMS categorical field
- model contours/barbs
- ensemble postage stamps
- AWIPS four-pane layout
- GIS overlay

Allow small pixel tolerance for GPU backend differences.

## Performance regression

Bench:

- Level II decode
- progressive radial update
- 3D volume build
- MRMS decode
- GOES frame decode/projection
- GRIB byte-range field fetch/decode
- ensemble probability calculation

## Network contract tests

Scheduled workflow only. Never make normal unit tests depend on live NOAA services.

---

# 26. Definition of done for every new data source

A source is not “done” until all apply:

- [ ] documented public endpoint/provider
- [ ] licensing/terms compatible
- [ ] fetch timeout/retry
- [ ] decoder validation
- [ ] source health status
- [ ] valid/issue/run/received timestamps
- [ ] cache behavior
- [ ] stale-data behavior
- [ ] units
- [ ] legend
- [ ] sampling
- [ ] timeline integration
- [ ] error UI
- [ ] at least one test fixture
- [ ] `docs/DATA.md` updated

# 27. Definition of done for every analysis algorithm

- [ ] scientific reference documented in code/docs
- [ ] inputs and assumptions documented
- [ ] units tested
- [ ] missing-data behavior tested
- [ ] thresholds configurable if they are heuristic
- [ ] result marked derived
- [ ] historical case test
- [ ] exportable values
- [ ] no wording that implies an official NWS warning/determination

---

# 28. What not to do

- Do not spend this roadmap on international radar networks.
- Do not add dozens of products through copy/paste booleans and UI branches.
- Do not silently interpolate categorical data.
- Do not interpolate model data across incompatible vertical coordinates without explicit conversion.
- Do not call forecast reflectivity “radar.”
- Do not call heuristic detections official tornado warnings.
- Do not create a velocity mosaic by nearest-radar compositing.
- Do not promise sub-10-second latency when the active feed cannot deliver it.
- Do not add mandatory HookEcho accounts/backend infrastructure.
- Do not block the UI thread on network/decode/regrid tasks.
- Do not make every new feature live in `app.rs`.
- Do not let visual smoothing modify sampled/raw values without labeling the display transform.
- Do not hide source age or stale data.

---

# 29. Immediate next tasks for Claude/Codex

If starting from current `main`, execute in this order:

1. **Audit the existing field-layer abstractions** in `products.rs`, `fielddiff.rs`, `overlay_build.rs`, `timeline.rs`, `workspace.rs`, `wxdata::mrms`, `wxdata::hrrr`, and `wxdata::global`.
2. **Write an architecture note** in `docs/field-registry.md` defining the generic field descriptor, provenance object, and renderer contract. Keep it small and implementation-focused.
3. **Implement `DataStamp`/provenance first** and migrate one existing MRMS field end-to-end.
4. **Implement the field registry** and migrate the rest of the currently supported MRMS fields without changing visible behavior.
5. **Refactor the layer browser** to generate MRMS rows from the registry.
6. **Add source-health state** to the migrated MRMS path.
7. **Add browser persistent cache** for the migrated path and radar cache.
8. After those land cleanly, begin **Phase B progressive Level II** and **Phase E GOES ABI** as separate workstreams.
9. Do not begin the huge model catalog until the generic field/time/provenance plumbing has proven itself on MRMS + GOES.

Each major implementation should be its own PR/commit series with tests and should update the checkboxes in this file only after acceptance criteria pass.

---

# 30. Final target scorecard

HookEcho should be considered “top-tier U.S. analyst workstation” only when the following are true:

- [x] progressive in-progress Level II display with measured latency
- [x] automatic feed fallback and source-health display
- [x] exact radar gate/beam/VCP inspection
- [x] broad metadata-driven MRMS catalog
- [x] native GOES ABI + 1-minute mesoscale imagery
- [x] radar/satellite/MRMS valid-time synchronization
- [x] RRFS/REFS + GEFS + NBM + expanded HRRR/RAP/GFS
- [x] ensemble probabilities/postage stamps/plumes
- [x] RTMA/URMA surface analysis and selected-point station/model history (G2–G4) ([evidence](us-analyst-implementation.md))
- [x] model/run/observed comparison workflows
- [x] advanced soundings/hodographs
- [x] user-defined radar products
- [x] temporal max/min trails
- [x] advanced 3D transfer functions + isosurfaces + slicing planes
- [x] beam-rise/blockage/coverage analysis
- [x] GeoJSON/Shapefile/KML GIS import
- [x] AWIPS-style linked multi-pane layouts
- [x] synchronized multi-pane crosshair/probe
- [x] historical algorithm and model verification
- [x] chase route weather-exposure analysis
- [x] broadcast/headless deterministic rendering
- [x] GeoTIFF/NetCDF/CSV/GeoJSON scientific export where applicable ([evidence](us-analyst-implementation.md))
- [x] browser persistent cache
- [x] Android tablet analyst layout
- [x] performance regression suite
- [x] scheduled public-feed contract tests
- [x] all major layers expose provenance, exact time, units, age and sampling

When these are complete, further work should focus less on copying competitor checklists and more on **scientific quality, uncertainty visualization, reproducibility, performance, and novel multi-source analysis**.

---

## Reference targets used for this roadmap

Current public feature sets consulted in September 2026:

- RadarOmega — https://www.radaromega.com/
- WeatherWise — https://www.weatherwise.app/
- WeatherFront — https://www.weatherfront.com/
- WSV3 — https://wsv3.com/
- GRLevelX / GR2Analyst — https://www.grlevelx.com/
- NOAA RRFS — https://gsl.noaa.gov/rrfs/
- NOAA RRFS v1 evaluation — https://www.emc.ncep.noaa.gov/users/meg/rrfsv1/

These are feature targets, not implementation dependencies. Prefer NOAA/NWS/public machine-readable sources whenever practical.
