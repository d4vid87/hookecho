//! External-process plugins: run a command, read a placefile off its stdout, draw it.
//!
//! The plugin API is a text format the app already renders and a handful of environment
//! variables. That buys language independence (a plugin is a Python script, a shell one-liner, a
//! Go binary), OS-level isolation, and zero new dependencies or binary weight — an embedded Lua
//! or WASM runtime would have cost a sandbox surface or tens of megabytes in the APK for a
//! capability nobody asked for yet. `// ponytail: mlua is the upgrade path if plugins ever need
//! in-process access to radar moments rather than just the view.`
//!
//! **Trust model:** the user typed the command, so it runs with the user's own privileges — the
//! same trust as a shell alias. The output cap and the timeout are hygiene against a plugin that
//! wedges or floods, not a claim that a hostile plugin is contained. Don't add plugins you
//! wouldn't run by hand.
//!
//! Desktop only: Android can't execute downloaded programs from app storage.

/// Largest stdout we'll read from a plugin. A placefile this size already has more items than
/// the map can usefully show; past it something has gone wrong and we stop rather than swell.
#[cfg(not(target_os = "android"))]
const MAX_OUTPUT: usize = 10 * 1024 * 1024;
/// How long a plugin gets before it's killed. Refresh cadences are seconds to minutes, so a
/// plugin that can't answer in ten seconds is hung, not slow.
#[cfg(not(target_os = "android"))]
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// What the app tells a plugin about the view it's drawing for.
#[derive(Debug, Clone, PartialEq)]
pub struct Context {
    pub site: String,
    /// `min_lon,min_lat,max_lon,max_lat`.
    pub bbox: (f64, f64, f64, f64),
    /// The instant on screen — the archive time when scrubbing, so a plugin can answer for then.
    pub time: chrono::DateTime<chrono::Utc>,
    pub product: String,
}

impl Context {
    /// The environment a plugin is handed. Names are part of the plugin API: don't rename them
    /// without a matching note in docs/plugins.md.
    pub fn env(&self) -> Vec<(&'static str, String)> {
        let (w, s, e, n) = self.bbox;
        vec![
            ("HOOKECHO_SITE", self.site.clone()),
            ("HOOKECHO_BBOX", format!("{w},{s},{e},{n}")),
            ("HOOKECHO_TIME", self.time.to_rfc3339()),
            ("HOOKECHO_PRODUCT", self.product.clone()),
        ]
    }
}

/// Run one plugin and parse its stdout as a placefile.
#[cfg(not(target_os = "android"))]
pub async fn run(
    command: &str,
    args: &[String],
    ctx: &Context,
) -> anyhow::Result<wxdata::placefile::Placefile> {
    use std::process::Stdio;
    use tokio::io::AsyncReadExt;

    let mut cmd = tokio::process::Command::new(command);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // A plugin that outlives its refresh would pile up copies of itself.
        .kill_on_drop(true);
    for (k, v) in ctx.env() {
        cmd.env(k, v);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("could not run {command}: {e}"))?;

    let mut out = Vec::new();
    let mut err = String::new();
    let read = async {
        if let Some(mut so) = child.stdout.take() {
            // `take` caps the read itself, so a plugin that never stops writing can't grow us.
            (&mut so).take(MAX_OUTPUT as u64).read_to_end(&mut out).await?;
        }
        if let Some(mut se) = child.stderr.take() {
            (&mut se).take(64 * 1024).read_to_string(&mut err).await?;
        }
        child.wait().await
    };
    let status = match tokio::time::timeout(TIMEOUT, read).await {
        Ok(r) => r?,
        Err(_) => {
            let _ = child.start_kill();
            anyhow::bail!("{command} did not finish within {}s", TIMEOUT.as_secs());
        }
    };
    if !status.success() {
        let tail = err.lines().next_back().unwrap_or("no output on stderr");
        anyhow::bail!("{command} exited with {status}: {tail}");
    }
    let text = String::from_utf8_lossy(&out);
    let pf = wxdata::placefile::parse(&text);
    // A plugin that runs cleanly but draws nothing is almost always a broken plugin, and saying
    // so beats an overlay that silently isn't there.
    anyhow::ensure!(
        !pf.items.is_empty(),
        "{command} produced no placefile items ({} bytes of output)",
        out.len()
    );
    Ok(pf)
}

/// Android has no way to execute a user-supplied program from app storage.
#[cfg(target_os = "android")]
pub async fn run(
    command: &str,
    _args: &[String],
    _ctx: &Context,
) -> anyhow::Result<wxdata::placefile::Placefile> {
    anyhow::bail!("plugins are desktop-only ({command} not run)")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(target_os = "android"))]
    use std::time::Duration;

    fn ctx() -> Context {
        Context {
            site: "KTLX".into(),
            bbox: (-98.0, 34.0, -96.0, 36.0),
            time: chrono::DateTime::parse_from_rfc3339("2013-05-20T20:15:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            product: "REF".into(),
        }
    }

    #[test]
    fn env_is_the_documented_shape() {
        let e = ctx().env();
        let get = |k: &str| e.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone());
        assert_eq!(get("HOOKECHO_SITE").unwrap(), "KTLX");
        assert_eq!(get("HOOKECHO_BBOX").unwrap(), "-98,34,-96,36");
        assert_eq!(get("HOOKECHO_TIME").unwrap(), "2013-05-20T20:15:00+00:00");
        assert_eq!(get("HOOKECHO_PRODUCT").unwrap(), "REF");
    }

    #[cfg(not(target_os = "android"))]
    #[tokio::test]
    async fn runs_a_plugin_and_reads_its_placefile() {
        let pf = run(
            "sh",
            &[
                "-c".into(),
                // Echoes the site back, which also proves the environment arrived.
                "printf 'Title: %s\\nColor: 255 0 0\\nLine: 2, 0\\n 35, -97\\n 36, -96\\nEnd:\\n' \
                 \"$HOOKECHO_SITE\""
                    .into(),
            ],
            &ctx(),
        )
        .await
        .unwrap();
        assert_eq!(pf.title, "KTLX");
        assert_eq!(pf.items.len(), 1);
    }

    #[cfg(not(target_os = "android"))]
    #[tokio::test]
    async fn a_hung_plugin_is_killed_not_waited_on() {
        let started = std::time::Instant::now();
        let err = run("sleep", &["600".into()], &ctx()).await.unwrap_err();
        assert!(err.to_string().contains("did not finish"), "{err}");
        assert!(started.elapsed() < TIMEOUT + Duration::from_secs(5));
    }

    #[cfg(not(target_os = "android"))]
    #[tokio::test]
    async fn a_failing_plugin_reports_its_stderr() {
        let err = run("sh", &["-c".into(), "echo boom >&2; exit 3".into()], &ctx())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("boom"), "{err}");
    }

    #[cfg(not(target_os = "android"))]
    #[tokio::test]
    async fn a_missing_command_is_an_error_not_a_panic() {
        assert!(run("hookecho-no-such-command", &[], &ctx()).await.is_err());
    }
}
