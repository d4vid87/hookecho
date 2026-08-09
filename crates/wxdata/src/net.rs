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
// ponytail: string surgery over a URL crate, and a short known-good list rather than a preflight
// probe; the ceiling is "the app's own feeds", and any host it cannot parse just goes unproxied.
pub fn fetch_url(url: &str) -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        url.to_string()
    }
    #[cfg(target_arch = "wasm32")]
    {
        /// Hosts that answer a cross-origin browser fetch on their own.
        const CORS_OK: &[&str] = &["api.open-meteo.com", "api.mapbox.com", "api.maptiler.com"];
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

#[cfg(test)]
mod tests {
    #[test]
    fn native_urls_are_left_alone() {
        let url = "https://api.weather.gov/alerts/active";
        assert_eq!(super::fetch_url(url), url);
    }
}
