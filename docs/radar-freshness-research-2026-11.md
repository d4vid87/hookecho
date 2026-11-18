# Radar freshness: external evidence, November 2026

Checked **2026-11-18**. This is a source review and a small endpoint-availability
check, not a measured competitor race. No paid application, private feed, or
competitor backend was exercised. Proposed benefits below are acceptance
measurements, not claimed results. HookEcho implementation and platform results
belong in the accompanying investigation report.

Evidence labels: **vendor** = documented by the application publisher;
**primary** = service/platform specification or radar authority;
**source** = public implementation inspected at a pinned revision;
**direct** = HTTP response observed here; **historical** = dated past behavior;
**hypothesis** = engineering inference that still needs a HookEcho experiment.
Effort is relative: S = localized behavior/test, M = several pipeline boundaries,
L = new operational service or substantial rendering work.

## Application evidence

| Application/source | Useful behavior and source date | URL | Checked | Quality | HookEcho application | Effort | Benefit to measure |
| --- | --- | --- | --- | --- | --- | --- | --- |
| RadarScope radar overview | Describes native-resolution rendering, product/tilt differences, and update checks tied to scan strategy. Page displays updated 2024-08-08. Its full-volume wording is not a specification for present-day chunk ingestion. | [Radar Data Overview](https://radarscope.zendesk.com/hc/en-us/articles/4411519606802-Radar-Data-Overview) | 2026-11-18 | Vendor; not runtime verification | Preserve scientific bins and compare identical products/tilts; do not claim that faster polling can create a newer observation. | S | Same acquisition identity and numerical values before/after. |
| RadarScope legend and controls | Product observation time is distinct from the last refresh/check time; stale times are highlighted. Direct product and tilt selection, interactive looping, and latest-frame access. Legend dated 2024-04-02; guide dated 2024-04-04. | [Product Legend](https://radarscope.zendesk.com/hc/en-us/articles/8989474231058-Product-Legend), [Getting Started](https://radarscope.zendesk.com/hc/en-us/articles/8970695921170-Getting-Started) | 2026-11-18 | Vendor | Give displayed observation age and last successful refresh separate meanings. Return to Latest must change follow state, not merely select an index. | S–M | Timestamp agrees with selected pixels; interaction count; stale/offline state survives failed refresh. |
| RadarScope reliability releases | App Store history lists 5.7.3 on Nov 5; 5.6.1 on Sep 2 includes slow-network download and watch-resume refresh fixes. 5.6 includes radar-switch download optimization, map-cache improvements, product labeling/unit fixes, and loop progress fixes. Latest entries omit year on the rendered store page. | [Developer release history](https://apps.apple.com/us/app/radarscope/id288419283) | 2026-11-18 | Vendor release notes | Use radar switching, weak networks, resume, wrong-product labels, and meaningful progress as regression scenarios. Release notes are not evidence of a proprietary delivery mechanism. | S–M | Resume-to-newest latency, failed-request recovery, wrong-site frames, time to first useful display. |
| RadarScope provider/map choices | Documents fallback when a requested product is unavailable from the selected provider; basic roads versus detailed Mapbox maps. Classic products are suggested for limited bandwidth. Provider page dated 2025-04-07; classic page 2024-08-12. | [Providers](https://radarscope.zendesk.com/hc/en-us/articles/4411492912146-Data-Provider-Settings), [Classic products](https://radarscope.zendesk.com/hc/en-us/articles/4411511509138-Classic-Products) | 2026-11-18 | Vendor | Make actual source/capability changes visible. Reduce optional basemap work before altering meteorological meaning. A lower-resolution product must remain explicitly named. | S–M | Download bytes and frame time with the same radar product; visible fallback identity. |
| RadarOmega | Base single-site workflow and 30-frame animation; tier-dependent dual/quad panes and longer history. Desktop requires a subscription; Windows/macOS/Linux downloads, no browser application. Undated landing page. | [Official capabilities/pricing](https://www.radaromega.com/) | 2026-11-18 | Vendor | Keep pane site/product/time labels visible and independent; compare equivalent modes and subscription capabilities. | S–M | Steps to compare two products; per-pane temporal mismatch and duplicate download count. |
| RadarOmega RapidSweep | Current guide identifies version 5.9. Pages 28–29 describe incremental Level II chunks, scan progress and selected-tilt beam visibility; the sweep-bar overlay can be hidden without disabling updates. WSR-88D and selected products only. | [Current official guide, pp. 28–29](https://www.sdsweather.com/PDF/RadarOmegaUserGuide.pdf), [Store/add-on listing](https://apps.apple.com/us/app/radaromega-doppler-radar-app/id1439881811) | 2026-11-18 | Vendor; document directly retrieved and relevant pages rendered | Separate ingestion from playback/visual effects. Distinguish a completed selected sweep from a completed volume. If showing partial sectors, expose their completeness and age. | M | Upstream-first-seen to first useful selected sweep, complete-sweep latency, partial-sector correctness. |
| GRLevelX / GR2Analyst | Main/product pages return 403. NOAA's 2014 training guide documents duplicate-tilt controls, polling configuration, and side-by-side supplemental-sweep comparison. An accessible publisher document for GR2Analyst 3 describes product/tilt selection and user-derived products computed at volume end. | [Publisher](https://www.grlevelx.com/), [GR2Analyst 3](https://www.grlevelx.com/gr2analyst_3/), [NOAA training, 2014-05-23](https://training.weather.gov/wdtd/buildTraining/Build14/documents/SAILSandGR2-5-23-2014.pdf), [Publisher UDP guide](https://www.grlevelx.com/downloads/UserDefinedProducts_3.pdf) | 2026-11-18 | Historical + limited vendor documentation; current general guide inaccessible | Retain repeated acquisitions at the same elevation and distinguish derived-volume products from early single sweeps. Do not apply historical polling defaults to current releases. | M | Supplemental-sweep count retained; chronological ordering; comparable observation times in panes. |
| Supercell Wx user guide | Live, Archive, and Auto Update are explicit states. Auto Update remains enabled during Live View animation and at the latest scan. Warns that switching between WSR-88D/TDWR can require selecting an available product. No publication date exposed. | [Radar Toolbox](https://supercell-wx.readthedocs.io/en/stable/user-guide/radar-toolbox.html) | 2026-11-18 | Vendor/open-source project documentation | Independent acquisition and selection state; preserve deliberate archive/scrub selection while ingesting updates; explicit unavailable-product handling. | S–M | New acquisitions arrive during playback; historical selection stays stable; no blank silent product failure. |
| Supercell Wx implementation | At commit `548ffc41727eb5f9d5e10e929d213ed9de7babac` (2026-11-05), timeline tracks update versus map-painted completion. Product manager reference-counts provider refresh use and checks product availability. This is inspected code, not proof of hardware presentation timing. | [Timeline manager](https://github.com/dpaulat/supercell-wx/blob/548ffc41727eb5f9d5e10e929d213ed9de7babac/scwx-qt/source/scwx/qt/manager/timeline_manager.cpp#L305-L361), [Provider refresh ownership](https://github.com/dpaulat/supercell-wx/blob/548ffc41727eb5f9d5e10e929d213ed9de7babac/scwx-qt/source/scwx/qt/manager/radar_product_manager.cpp#L458-L513) | 2026-11-18 | Source | Useful design references for shared site ingestion and frame-associated diagnostics. Reuse HookEcho's existing ownership/diagnostics first. | M | One fetch/decode per shared acquisition; decoded/uploaded/presented stages remain distinguishable. |
| RainViewer | Maps API serves two hours at ten-minute frame intervals. `generated` describes manifest creation; frame `time` describes mosaic generation, not all contributing observations. Use returned `host` and opaque `path`. | [Weather Maps API](https://www.rainviewer.com/api/weather-maps-api.html) | 2026-11-18 | Primary + direct manifest check below | Label mosaic generation time honestly; do not compare it directly with a selected station sweep timestamp. Avoid constructing paths from timestamps. | S | Manifest/frame/pixel identity survives API updates; timestamp type visible. |
| Windy timestamp discussion | On 2024-02-06, staff explained that identical imagery had different displayed times after switching from latest-frame update time to five-minute slots. Topic/API still readable, with a deleted-topic banner. | [Historical discussion](https://community.windy.com/topic/31271/radar-slider/7) | 2026-11-18 | Historical staff explanation | Audit imagery identity alongside metadata. This establishes a timestamp failure mode, not current Windy speed or behavior. | S | Identical images cannot appear newer solely because a poll/slot clock changed. |

RadarScope's public materials reviewed here did **not** substantiate pushed radar
updates, a particular socket protocol, or a private backend architecture. They
also did not establish vector-map implementation or measured map bandwidth.
Do not turn those investigation prompts into findings. Its widget documentation
explicitly makes refresh scheduling subject to the OS, which is distinct from
foreground application behavior. [Widget documentation](https://radarscope.zendesk.com/hc/en-us/articles/8991265059346-Home-Screen-Widget)

RadarOmega's guide claims a 35-second to two-minute advantage over other
providers. No matched observation or paid runtime test was performed here, so
this is only an unverified vendor comparison. The useful transferable feature
is incremental data availability, not the advertised numerical advantage.

## Data and platform evidence

| Source | Useful behavior / URL | Checked | Quality | HookEcho application | Effort | Benefit to measure |
| --- | --- | --- | --- | --- | --- | --- |
| NOAA SAILS | Up to three supplemental lowest-elevation scans within one volume. [ROC SAILS](https://www.roc.noaa.gov/radar-techniques/sails.php) | 2026-11-18 | Primary | A volume/elevation pair is not a unique acquisition. Preserve sweep/radial time and sequence for supplemental and split cuts. | M | All valid repeated sweeps survive decode, cache, timeline, and rendering. |
| AWS NEXRAD registry | Current archive bucket is `unidata-nexrad-level2`; old archive retired 2025-09-01. Lists live chunks, Level III, and current notification topics. [Registry](https://registry.opendata.aws/noaa-nexrad/) | 2026-11-18 | Primary + direct endpoint check | Check current endpoint configuration and documentation. Keep archive and chunk semantics distinct. | S | No calls to retired archive; current-site objects discoverable. |
| AWS chunk-format documentation | Chunk path carries site, rolling volume number, volume date/time, chunk number/type; volumes cycle 0–999. Filterable notifications expose identity fields. Archive bucket/topic examples on this page are stale. [AWS source documentation](https://github.com/awslabs/open-data-docs/tree/main/docs/noaa/noaa-nexrad) | 2026-11-18 | Primary, partly stale | Include date/time in volume identity; reorder by sequence and deduplicate; do not sort rolling volume directories to find latest. Prefer registry for current endpoints. | M | Wrap, duplicates, missing chunks, late data, UTC rollover fixtures. |
| xradar 0.12.0 | Streaming example keeps completed sweeps, drops incomplete sweeps by default, or represents missing rays with NaN when padding. [Chunk guide](https://docs.openradarscience.org/projects/xradar/en/v0.12.0/notebooks/nexrad_read_chunks.html) | 2026-11-18 | Public technical example; not executed | Expose completed useful sweeps early; treat missing sectors as missing. Its old archive name and illustrative filename/volume-selection examples must not be copied as current endpoint/order contracts. | M | Selected sweep available before full volume; no invented observations in missing sectors. |
| MRMS | Registry documents a two-minute delivery cycle and current `noaa-mrms-pds` bucket. [MRMS registry](https://registry.opendata.aws/noaa-mrms-pds/) | 2026-11-18 | Primary + direct endpoint check | Cadence is not constituent observation age. Compare mosaics with mosaics and retain product valid/generation semantics. | S–M | Generation, arrival, and displayed timestamps agree; per-product availability measured. |
| HTTP caching | `no-cache` permits storage but requires revalidation; `no-store` forbids storage. Long-lived immutable caching belongs to unchanged content. History restoration can bypass a new HTTP request. [MDN caching](https://developer.mozilla.org/en-US/docs/Web/HTTP/Guides/Caching) | 2026-11-18 | Platform guidance | Distinguish mutable listings/latest pointers from known complete immutable objects; preserve validators and useful history caching. A filename alone does not prove an object is immutable. | S–M | Validator use, repeated bytes, cache age, newest-object change visibility, no persistent cached failure. |
| Browser visibility/lifecycle | Visibility changes are observable; background scheduling is constrained. Frozen pages suspend timers/fetch callbacks; discarded pages lose execution. [MDN visibility](https://developer.mozilla.org/en-US/docs/Web/API/Page_Visibility_API), [Chrome lifecycle](https://developer.chrome.com/docs/web-platform/page-lifecycle-api) | 2026-11-18 | Platform guidance | Reconcile freshness on visible/resume/history restoration rather than relying on background polling deadlines. Preserve user selection and last-good view. | S–M | Foreground-to-newest delay and clean reconnection after hide/freeze/offline. |
| S3 notifications | At-least-once delivery, possible duplicates/out-of-order events; usually seconds, sometimes a minute or more. [Notifications](https://docs.aws.amazon.com/AmazonS3/latest/userguide/EventNotifications.html), [Ordering](https://docs.aws.amazon.com/AmazonS3/latest/userguide/notification-how-to-event-types-and-destinations.html#event-ordering-and-duplicate-events) | 2026-11-18 | Primary | Push needs idempotency, bounded queues, reconnect listing reconciliation, and age guards. Notification receipt is not observation time. | L | Discovery latency distribution and missed/duplicate/out-of-order recovery, including service failure. |
| Android ADPF | Thermal APIs support sustainable workload adjustment; fixed-performance mode is a benchmark aid. Guidance last updated 2026-02-26. [Android ADPF](https://developer.android.com/games/optimize/adpf) | 2026-11-18 | Platform guidance | Measure sustained physical-device load before adding thermal integration; reduce optional visual work rather than radar correctness. | M | Long-session frame time, temperature/thermal status, battery, memory, newest-observation delay. |

## Endpoint availability observations

Unauthenticated GET probes began **2026-11-18 21:45:13.969 UTC** on this Linux
host. S3 response dates were 21:45:15 UTC. These are six small availability
probes, not p50/p95 latency samples and not proof of newest observation age.
List requests were bounded and generally return the lexicographically first
matching objects. `LastModified` below is object metadata, never observation
time.

| Request | Direct result |
| --- | --- |
| [Current Level II archive](https://unidata-nexrad-level2.s3.amazonaws.com/?list-type=2&prefix=2026/11/18/KTLX/&max-keys=3) | HTTP 200; first key `2026/11/18/KTLX/KTLX20261118_000036_V06`; size 2,062,320 bytes; object LastModified `00:07:22Z`. |
| [Level II chunks](https://unidata-nexrad-level2-chunks.s3.amazonaws.com/?list-type=2&prefix=KTLX/&max-keys=3) | HTTP 200; keys begin `KTLX/1/20261118-141833-001-S`, then `002-I`, `003-I`. This bounded listing does not identify the latest rolling volume. |
| [Level III bucket](https://unidata-nexrad-level3.s3.amazonaws.com/?list-type=2&max-keys=1) | HTTP 200; one historical object listed. Only bucket availability tested, not latest Level III availability. |
| [MRMS composite date prefix](https://noaa-mrms-pds.s3.amazonaws.com/?list-type=2&prefix=CONUS/MergedReflectivityQCComposite_00.50/20261118/&max-keys=3) | HTTP 200; first three keys include `20261118-000038`, `000238`, `000436`. No GRIB observation fields decoded in this probe. |
| [Retired Level II archive](https://noaa-nexrad-level2.s3.amazonaws.com/?list-type=2&max-keys=1) | HTTP 403, consistent with registry retirement notice. |
| [RainViewer current manifest](https://api.rainviewer.com/public/weather-maps.json) | HTTP 200, 818 bytes, `Cache-Control: no-cache`; 13 past frames spaced 600 seconds; `generated=1795038028`; final frame `time=1795038000`, `path=/v2/radar/bff4bb9534fb`; `nowcast=[]`, `satellite.infrared=[]`. Tile content was not downloaded. |

## Access, licensing, rates, cost, and support

| Source | Constraints checked on 2026-11-18 |
| --- | --- |
| RadarScope | US store base price $9.99 with in-app purchases; dual/multiple panes and other features vary by Pro tier/platform. Proprietary application/feed access is not a reusable API license. Support via linked Zendesk help center. Paid capabilities untested. [Store](https://apps.apple.com/us/app/radarscope/id288419283) |
| RadarOmega | Site lists $8.99 mobile base; Gamma/Beta/Alpha $4.99/$8.99/$11.99 monthly and $49/$89/$119 annually. Store lists RapidSweep add-ons $7.99 and $84.99 without term labels in the extracted list. Prices/store variants differ. No HookEcho feed access assumed; support address on homepage. [Site](https://www.radaromega.com/), [Store](https://apps.apple.com/us/app/radaromega-doppler-radar-app/id1439881811) |
| GRLevelX | Current price/license/support terms could not be verified from the blocked product pages. No purchase, installation, owner forum access, or commercial-feed test. An unaffiliated users' forum was not treated as current vendor authority. |
| Supercell Wx / xradar | MIT-licensed code; retaining required notices matters if substantial code is copied. No source was copied into HookEcho. Supercell is free; selected map providers have separate terms. Community support is not an availability SLA. [Supercell license](https://github.com/dpaulat/supercell-wx/blob/548ffc41727eb5f9d5e10e929d213ed9de7babac/LICENSE.txt), [xradar license](https://github.com/openradar/xradar/blob/main/LICENSE), [Supercell FAQ](https://supercell-wx.readthedocs.io/en/stable/getting-started/faq.html) |
| NOAA NEXRAD / MRMS | Public unsigned bucket access; NOAA requests attribution and forbids implying endorsement or presenting altered data as original. Registry pages specify no per-client request quota or latency SLA. NEXRAD support: `support-level2@unidata.ucar.edu`; MRMS: `mrms@noaa.gov`, delivery: `nodd@noaa.gov`. Public data does not make a new ingestion/fan-out service cost-free. [NEXRAD registry](https://registry.opendata.aws/noaa-nexrad/), [MRMS registry](https://registry.opendata.aws/noaa-mrms-pds/) |
| RainViewer | Free service described for personal/educational/small community uses, mandatory visible linked attribution, no SLA; commercial/high-volume uses need bespoke terms. **Docs conflict:** landing FAQ says no posted hard limit and mentions nowcasts; transition page says 100 requests/IP/minute, max zoom 7, no nowcast/IR since 2026-01-01. Use conservative transition limits and live manifest capabilities; obtain provider clarification before relying on broader rights/capacity. Support: `support@rainviewer.com`. [Terms/FAQ](https://www.rainviewer.com/api.html), [Transition](https://www.rainviewer.com/api/transition-faq.html) |
| Notifications/shared service | NEXRAD publishes filterable chunk and archive topics; MRMS permits SQS/Lambda subscribers. AWS account, consumer queue/compute, retries, logging, storage and client fan-out create operating responsibility. SQS bills API actions/payload units and advertises 1 million free requests/month; SNS costs depend on delivery/filtering/transfer. No fixed monthly total is defensible without event volume, clients and retention. [SQS pricing](https://aws.amazon.com/sqs/pricing/), [SNS pricing](https://aws.amazon.com/sns/pricing/) |

The current RadarOmega guide was downloaded from the homepage's linked URL:
HTTP 200, 49,799,960 bytes, `Last-Modified: 2026-11-09 18:14:10 GMT`;
cover says version 5.9. PDF pages 31–32 (printed 28–29) were rendered and
inspected. Older `RO_UserGuide_2024NEW.pdf` and `RO_UserGuide_2025NEW.pdf`
links returned 404 despite search-index excerpts. The web text extractor could
not open the replacement; a direct HTTP retrieval succeeded. GRLevelX main
and product pages returned 403 in both the web tool and direct HTTP; its UDP
PDF was accessible. Windy post date was cross-checked through its public topic
API. Dates rendered inside vendor articles can differ from CMS update metadata;
the table uses the visible article date, not an inferred publication history.

## Changes and experiments justified by this evidence

These are engineering hypotheses until linked to HookEcho tests and traces:

1. **Keep ingestion independent of animation.** Compare paused-live versus
   live-loop on the same site/moment/elevation. Assert equal acquisition intake,
   no forced jump while historically scrubbed, and explicit loop-head admission.
2. **Associate age with the displayed sweep.** Track acquisition range/identity,
   completeness, actual source, and frame identity through presentation.
   A refresh, decode completion, GPU upload, or UI repaint cannot renew weather
   observation age. Mark absent source-publication/presentation timestamps unknown.
3. **Reconcile on foreground/network recovery.** Request current availability
   once through existing ownership guards, avoiding duplicate retries. Test
   hidden/frozen/history-restored web pages and Android/desktop resume separately.
4. **Revalidate mutable discovery without discarding history caching.** Record
   the cache policy at each actual request layer and verify whether a newest
   object can change. Do not claim an incomplete-object cache bug without observing
   changing content or proving it with a fixture.
5. **Retain distinct supplemental sweeps.** Test SAILS, split cuts, volume wrap,
   gaps, duplicates, reordering and completed-volume reconciliation. Prefer a
   complete selected sweep early; partial sectors require visible missingness.
6. **Measure before splitting workers or adding a push service.** Compare
   newest-radar queue delay under identical heavy-layer load. A service is
   justified only if measured polling/discovery or redundant-client work dominates
   after bounded client fixes; retain polling reconciliation as rollback.

For periodic polling with period T and independent uniformly phased upstream
arrivals, expected discovery wait is T/2 and the ideal bound is T. That is a
model, not a field result: caches, retries, visibility suspension and queueing
violate its assumptions. Report stages separately rather than summing overlapping
timers. A defensible before/after study needs multiple acquisitions, synchronized
UTC, monotonic local durations, matched product identities, and both freshness
and resource costs.

Not checked: competitor runtime speed, paid feeds, private backend behavior,
physical Android thermal behavior, or notification subscription delivery. No
subscriptions, infrastructure, messages, publication, or deployment were created.
