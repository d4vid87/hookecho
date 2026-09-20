# Field registry

The registry gives gridded products stable identities and one metadata contract while existing
renderers migrate incrementally. It lives in `wxdata::field`; UI and GPU types stay in `hookecho`.

## Ownership

- `FieldDescriptor` is static product metadata: identity, source, family, units, value kind,
  palette, search aliases, sampling, missing-data behavior, and supported operations.
- `GridSpec` describes the decoded native grid. Unknown resolution remains `None`.
- `DataStamp` records source-object identity and issue/run/valid/received times. Cache access time
  is separate and must never replace `received_time`.
- `FieldFrame` owns immutable native decoded values and provides the common sampling entry point.
- `SampleResult` carries the sampled value, units, quality, exact valid time, and method.

Stable IDs use lowercase namespaced strings such as `mrms.composite-reflectivity`. Display names
may change; persisted IDs may not. Unknown persisted IDs are skipped without rejecting the rest of
the settings file.

## Compatibility

Registry-backed frames wrap the existing `MrmsField`, so proven GRIB decoding and the current GPU
upload path remain unchanged. `OverlayMsg::RegisteredField` is the migration bridge. Unmigrated
products continue through `OverlayMsg::Field` and `FieldLayer` until their descriptors exist.

Native values remain in `FieldFrame`; palette indexes, smoothing, and reduced GPU textures are
display products only. Continuous fields may opt into bilinear sampling. Categorical fields must
use nearest-neighbor sampling.

## First migration

`mrms.composite-reflectivity` was the first complete slice. Every currently supported MRMS field
now has a descriptor and enters through the same frame/provenance path. Acquisition records the
immutable S3 object key and receipt time, the renderer consumes native fields through the
compatibility bridge, and Layer options → Data details exposes source, valid/received times,
issue/run availability, classification, quality, units, grid, native resolution, sampling method,
and source identity. Descriptor IDs select source objects through the MRMS registry; the app no
longer carries a second product-path table. The common sampler reads native values rather than
the display texture.

HRRR composite reflectivity is the first model migration. It uses the same native-value frame and
sampler while retaining its existing forecast timeline and renderer. Its stamp identifies the
model object and exposes run, valid, receipt, forecast classification, and unknown quality.
HRRR/RAP CAPE and storm-relative helicity use the same path; their stamps distinguish forecast
guidance from RAP analysis while their existing source and layer controls remain intact.
The remaining existing regional guidance grids—HRRR updraft-helicity swaths, snowfall and smoke,
plus NBM thunder probability—also carry registry metadata without changing their controls.

The six existing global guidance layers use the same path. Their stable IDs are namespaced under
`model.global`, and each frame retains its GFS or ECMWF source object, run, valid, and receipt
times. The shared moisture layer requests total-column water from both providers so comparisons
do not mix GFS precipitable water with ECMWF accumulated precipitation.

NOHRSC observed snowfall is registry-backed as an analysis accumulation. The frame keeps the
selected 6/24/48/72-hour source object and its issue, valid, and receipt times.

Model differences are derived registry frames. They require matching valid times and retain both
contributor identities, the newest receipt time, native difference units, and derived status.
