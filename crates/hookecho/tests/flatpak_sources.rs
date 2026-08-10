//! `packaging/flatpak/cargo-sources.json` is generated from `Cargo.lock` and then committed, so
//! it goes stale the moment a dependency moves and nobody notices until a Flathub build fails.
//!
//! This is the notice: every registry crate in the lockfile must have a source entry. Regenerate
//! with `flatpak-cargo-generator.py Cargo.lock -o packaging/flatpak/cargo-sources.json`.

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is crates/hookecho.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// `name` + `version` of every crate in the lockfile that comes from a registry.
///
/// Line-wise rather than a TOML dependency: the lock format is `[[package]]` blocks of plain
/// `key = "value"`, and a package without a `source` is one of ours (a workspace member or a
/// vendored patch), which has no crates.io tarball to fetch.
fn locked_registry_crates(lock: &str) -> Vec<String> {
    let mut out = Vec::new();
    for block in lock.split("[[package]]").skip(1) {
        let field = |key: &str| {
            block
                .lines()
                .find_map(|l| l.strip_prefix(key)?.split('"').nth(1))
                .map(str::to_string)
        };
        if let (Some(name), Some(version), Some(_)) = (
            field("name = "),
            field("version = "),
            field("source = "),
        ) {
            out.push(format!("{name}-{version}"));
        }
    }
    out
}

#[test]
fn cargo_sources_covers_the_lockfile() {
    let root = repo_root();
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).expect("Cargo.lock");
    let sources =
        std::fs::read_to_string(root.join("packaging/flatpak/cargo-sources.json")).expect("sources");

    let missing: Vec<String> = locked_registry_crates(&lock)
        .into_iter()
        .filter(|c| !sources.contains(&format!("/{c}.crate")))
        .collect();

    assert!(
        missing.is_empty(),
        "packaging/flatpak/cargo-sources.json is stale — missing {} crate(s): {}\n\
         Regenerate: flatpak-cargo-generator.py Cargo.lock -o packaging/flatpak/cargo-sources.json",
        missing.len(),
        missing.join(", ")
    );
}

#[test]
fn the_lockfile_parser_finds_registry_crates_and_skips_ours() {
    let lock = r#"
[[package]]
name = "hookecho"
version = "0.8.0"
dependencies = ["wxdata"]

[[package]]
name = "serde"
version = "1.0.200"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "deadbeef"
"#;
    assert_eq!(locked_registry_crates(lock), vec!["serde-1.0.200"]);
}
