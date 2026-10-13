# REA research integration

This applies observed rendering and update patterns to HookEcho's own public-data
pipeline. No RadarOmega client code, credentials, private service adapter, or
proprietary palette is imported.

This records the initial integration and its verification revision. Subsequent custom-product precision, GPU globe wind, performance measurements and pane-independence changes are documented in [the performance audit](rea-performance-audit.md) and [the latest follow-up report](rea-next-improvements.md).

## Features

1. **Two-ended display filters:** the existing physical minimum and a pane-local
   maximum mask the palette, including precipitation-tinted rows. Native samples,
   probes and scientific analyses retain their original values. Custom products
   do not inherit native-moment bounds.
2. **Adjustable smoothing:** each pane has its own sharpened-bilinear blend width.
   Optional two-stage smoothing evaluates separable normalized gate/radial passes
   on demand, without an intermediate full-sweep texture. Missing/folded codes are
   excluded from numeric blending. This costs extra texture samples and stays off
   by default; Crisp retains nearest-gate sampling.
3. **Partial GPU uploads:** changed radial rows are coalesced into adjacent spans.
   Unchanged bytes require no texture writes; dense or fragmented updates fall
   back to one full write. Cleared rows and both R8/R16 textures are supported.
   No measured throughput improvement is claimed.
4. **Saved pane appearance:** workspaces embed independent `.pal` tables,
   smoothing, bounds, precision and projection. Older snapshots use the original
   defaults. Desktop/mobile legends select the pane's palette.
5. **Higher-precision display:** optional R16Uint gates are encoded directly from
   physical native values, derived KDP or unfolded velocity, with a 1024-entry
   palette. They are not upscaled R8 values. Missing/folded classifications are
   reserved; the binned cache accounts for additional bytes. Custom products keep
   their existing encoding and disable this control. Native probes are unchanged.
6. **Globe projection:** an optional orthographic sphere shares geographic math
   with CPU picking. Radar and gridded fields inverse-project each fragment;
   raster tiles use tessellated geometry and geographic UV lookup. Vector geometry
   clips the far hemisphere, and CPU feature edges clip at the horizon. Sky clicks
   are rejected, longitude wraps, and zoom preserves the cursor anchor. Tile
   requests are bounded; wind uses the CPU path in globe mode.

Appearance and projection controls are in **Search → Map settings** for the active pane.
Display bounds remain in the product controls. Save a workspace to retain them.
The default remains Mercator with the existing R8 display and smoothing behavior.

## Limits

- Globe basemaps retain their providers' Mercator coverage (about ±85° latitude).
  Polar caps have no invented imagery. Tile and vector horizon edges use finite
  tessellation, while weather sampling and picking use the inverse sphere.
- Extra smoothing and precision are optional resource costs. This does not increase
  source resolution, add new data services, or change scientific measurements.
- Saved workspace appearance is portable; proprietary vendor services and assets
  are not part of this integration.

## Verification

- Full workspace tests pass: 542 HookEcho library tests, 425 wxdata tests, and the
  remaining workspace/integration suites. Network/GPU tests stay explicitly ignored
  in this command.
- Seven hardware GPU checks pass separately (six rendering tests plus the cache
  reuse test): existing radar golden image and stroke/
  wind checks, changed-row/full-upload equivalence, R8→R16 format changes plus
  precision partial uploads, and globe grid/tile/picking alignment and front/back
  vector visibility. The smoothing comparison verifies the optional stages visibly
  affect the fixture; partial updates still match a fresh render.
- Workspace tests cover legacy defaults and restored pane appearance/projection.
  Native precision tests distinguish measurements that collide in R8 and preserve
  classifications. Globe tests cover sky rejection, wrapping, cursor anchoring and
  CPU edge clipping, local-scale continuity, zooming out toward sky and preserving
  projection when recentering on a station.
- Workspace Clippy passes with warnings denied. Browser proxy/decode logic: 22 pass.
- Production web build passes: main wasm 4,352,707 bytes gzip, lite wasm 74,040.
  The main budget is deliberately updated to 4,365,000 for the complete feature
  set (about 17 KB / 0.4% above the prior measured baseline); the lite budget and
  45-second boot limit are unchanged.
- Normal, forced-WebGL and older-browser/Family Hub boot checks pass. A real
  Chromium WebGL fixture run renders precision + globe + two-stage smoothing and
  saves those settings with blend width 0.73. The lite viewer also paints its
  repository velocity fixture and completes its loop smoke check.
- Browser screenshots use repository radar fixtures with intercepted test HTTP
  responses. They are validation captures, not live weather. Other feeds are
  unavailable on the static test server; live vendor endpoints are not exercised.
