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

/// Remembered HTTP validators, so a feed that has not changed can answer in a header instead of a
/// body.
///
/// One store, keyed by URL, holding whatever the last response offered (`ETag`, `Last-Modified`)
/// plus a caller's own tag for *what those bytes were* — a 304 says "the file has not changed
/// since you last read it", which only means "you are up to date" if what you hold is what that
/// read produced.
///
/// Native only. In a browser the same job is already done, better, by the HTTP cache and by the
/// edge in front of `/proxy/`; adding a second layer inside the wasm would cost bundle size to
/// re-implement what the platform does for free.
///
// ponytail: one adopter today (the DWD volume probe, where it turns a 201 KB poll into a
// header exchange). It lives here rather than in `dwd.rs` because the next feed that wants it
// should not write a second one — but it is deliberately small, and it is not a cache: it stores
// validators, never bodies.
#[cfg(not(target_arch = "wasm32"))]
pub mod validators {
    use std::num::NonZeroUsize;
    use std::sync::Mutex;

    /// What we remember about one URL.
    #[derive(Clone, Default)]
    pub struct Entry {
        pub etag: Option<String>,
        pub last_modified: Option<String>,
        /// The caller's name for the bytes that response carried.
        pub tag: Option<String>,
    }

    /// Bounded because the keys are URLs and a long session touches many. 256 covers every feed
    /// URL the app polls, several times over.
    fn store() -> &'static Mutex<lru::LruCache<String, Entry>> {
        static STORE: std::sync::OnceLock<Mutex<lru::LruCache<String, Entry>>> =
            std::sync::OnceLock::new();
        STORE.get_or_init(|| {
            Mutex::new(lru::LruCache::new(NonZeroUsize::new(256).unwrap()))
        })
    }

    /// What is remembered for `url`, if anything.
    pub fn get(url: &str) -> Option<Entry> {
        store().lock().ok()?.get(url).cloned()
    }

    /// Add `If-None-Match` / `If-Modified-Since` for `url`, if anything is remembered for it.
    ///
    /// Nothing remembered means the request goes out exactly as it would have.
    pub fn apply(req: reqwest::RequestBuilder, url: &str) -> reqwest::RequestBuilder {
        let Some(e) = get(url) else { return req };
        let req = match &e.etag {
            Some(v) => req.header(reqwest::header::IF_NONE_MATCH, v),
            None => req,
        };
        match &e.last_modified {
            Some(v) => req.header(reqwest::header::IF_MODIFIED_SINCE, v),
            None => req,
        }
    }

    /// Remember what a response offered for `url`, tagged with the caller's name for those bytes.
    /// Headers rather than the response itself, because the caller needs the body.
    ///
    /// A response with no validators clears the entry rather than leaving a stale one: sending a
    /// condition the server never issued is how a feed that stopped publishing them starts
    /// answering 304 to a request that should have been unconditional.
    pub fn remember_headers(url: &str, headers: &reqwest::header::HeaderMap, tag: Option<String>) {
        let header = |name: reqwest::header::HeaderName| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let entry = Entry {
            etag: header(reqwest::header::ETAG),
            last_modified: header(reqwest::header::LAST_MODIFIED),
            tag,
        };
        if let Ok(mut s) = store().lock() {
            if entry.etag.is_none() && entry.last_modified.is_none() {
                s.pop(url);
            } else {
                s.put(url.to_owned(), entry);
            }
        }
    }

    /// Forget everything. Tests only — the store is process-wide.
    #[cfg(test)]
    pub fn clear() {
        if let Ok(mut s) = store().lock() {
            s.clear();
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod validator_tests {
    use super::validators;

    fn headers(pairs: &[(&str, &str)]) -> reqwest::header::HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                reqwest::header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    /// What is remembered is what comes back, tagged with the caller's name for those bytes —
    /// the tag is the whole reason a 304 can be trusted to mean "you are up to date".
    #[test]
    fn remembers_validators_and_the_tag() {
        validators::clear();
        let url = "https://example.invalid/one";
        validators::remember_headers(
            url,
            &headers(&[("etag", "\"abc\""), ("last-modified", "Sat, 29 Aug 2026 10:00:00 GMT")]),
            Some("VOL-1".into()),
        );
        let e = validators::get(url).expect("remembered");
        assert_eq!(e.etag.as_deref(), Some("\"abc\""));
        assert_eq!(e.last_modified.as_deref(), Some("Sat, 29 Aug 2026 10:00:00 GMT"));
        assert_eq!(e.tag.as_deref(), Some("VOL-1"));
    }

    /// A response that carries no validators must clear what was remembered, not leave it.
    /// Otherwise a feed that stops issuing them starts answering 304 to a request that should
    /// have been unconditional — and the app stops seeing new data with nothing to show for it.
    #[test]
    fn a_response_without_validators_forgets_the_old_ones() {
        validators::clear();
        let url = "https://example.invalid/two";
        validators::remember_headers(url, &headers(&[("etag", "\"abc\"")]), Some("VOL-1".into()));
        assert!(validators::get(url).is_some());
        validators::remember_headers(url, &headers(&[("content-type", "text/plain")]), None);
        assert!(
            validators::get(url).is_none(),
            "a validator-less response must clear the entry"
        );
    }

    /// Nothing remembered means the request goes out exactly as it would have.
    #[test]
    fn an_unknown_url_adds_no_headers() {
        validators::clear();
        let client = reqwest::Client::new();
        let req = validators::apply(
            client.get("https://example.invalid/three"),
            "https://example.invalid/three",
        );
        let built = req.build().unwrap();
        assert!(built.headers().get(reqwest::header::IF_NONE_MATCH).is_none());
        assert!(built
            .headers()
            .get(reqwest::header::IF_MODIFIED_SINCE)
            .is_none());
    }
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
