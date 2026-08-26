//! The one place a feed URL is bent to fit the target it is fetched from.
//!
//! Native builds talk to NOAA directly. The browser build cannot: almost nothing on the feed list
//! sends an `Access-Control-Allow-Origin`, so the page asks its own origin instead and the
//! `--serve` CORS proxy (`hookecho::serve`) does the fetch. Pass every feed URL through
//! [`fetch_url`] and the difference stays here rather than in fifty call sites.

/// The URL to actually fetch for `url` on this target.
///
/// Native: unchanged, always. Wasm: rewritten to `{origin}/proxy/{host}/{path}` unless the host is
/// one of the few that sends CORS headers of its own. The rewrite is best-effort — anything that
/// does not parse as `https://host/...`, and any page with no reachable `window`, is left alone
/// and fails the same way it would today.
///
/// Hosts that answer a cross-origin browser fetch on their own, so the browser build asks them
/// directly instead of going through `/proxy/`.
///
/// The two keyed tile providers are here for a second reason as well: their tile URLs carry the
/// user's API key in the query string, and the proxy is a *shared* edge cache. Routing them
/// through it would store one user's key against a cacheable URL. They must stay direct.
// The live-chunk bucket is here rather than proxied on purpose: those objects are seconds old and
// fetched one at a time as the radar writes them, so an extra hop buys nothing and costs latency.
// The *archive* bucket is not in this list — it goes through the proxy, so one visitor's volume
// download is the next visitor's cache hit.
pub const CORS_OK: &[&str] = &[
    "api.open-meteo.com",
    "api.mapbox.com",
    "api.maptiler.com",
    // Same reason as the tile providers: the request carries the user's Synoptic token, and the
    // proxy is a shared cache. Synoptic sends `Access-Control-Allow-Origin: *`, so this works.
    "api.synopticdata.com",
    "unidata-nexrad-level2-chunks.s3.amazonaws.com",
];

// ponytail: string surgery over a URL crate, and a short known-good list rather than a preflight
// probe; the ceiling is "the app's own feeds", and any host it cannot parse just goes unproxied.
pub fn fetch_url(url: &str) -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        url.to_string()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let Some(rest) = url.strip_prefix("https://") else {
            return url.to_string();
        };
        let (host, path) = match rest.split_once('/') {
            Some(pair) => pair,
            None => (rest, ""),
        };
        if CORS_OK.contains(&host) {
            return url.to_string();
        }
        let Some(origin) = web_sys::window().and_then(|w| w.location().origin().ok()) else {
            return url.to_string();
        };
        format!("{origin}/proxy/{host}/{path}")
    }
}

/// Route nexrad-data's S3 fetches through [`fetch_url`] as well.
///
/// That crate builds its own bucket URLs and hands them straight to reqwest, so it was the one
/// feed path the proxy never saw. Call once at browser startup.
#[cfg(target_arch = "wasm32")]
pub fn install_s3_proxy_rewriter() {
    nexrad_data::aws::client::set_url_rewriter(fetch_url);
}

#[cfg(test)]
mod tests {
    /// A keyed tile URL must never be rewritten to `/proxy/…`: the proxy is a shared edge cache
    /// and the key rides in the query string.
    #[test]
    fn keyed_tile_hosts_are_never_proxied() {
        for host in ["api.mapbox.com", "api.maptiler.com"] {
            assert!(
                super::CORS_OK.contains(&host),
                "{host} must stay direct — proxying it would cache a user's API key"
            );
        }
    }

    #[test]
    fn native_urls_are_left_alone() {
        let url = "https://api.weather.gov/alerts/active";
        assert_eq!(super::fetch_url(url), url);
    }
}
