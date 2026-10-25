fn main() {
    let revision = std::process::Command::new("git").args(["describe", "--always", "--dirty", "--exclude", "*"])
        .output().ok().filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_owned()).unwrap_or_else(|| "unknown".into());
    let epoch = std::env::var("SOURCE_DATE_EPOCH").ok().and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    println!("cargo:rustc-env=HOOKECHO_BUILD_REVISION={revision}");
    println!("cargo:rustc-env=HOOKECHO_BUILD_DATE={epoch}");
    println!("cargo:rustc-env=HOOKECHO_BUILD_CHANNEL={}", if version.contains('-') { "Preview" } else { "Stable" });
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    // Track the worktree's HEAD/ref as well as source edits, so rebuilt binaries identify their commit.
    let reference = std::process::Command::new("git").args(["symbolic-ref", "-q", "HEAD"])
        .output().ok().filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok()).unwrap_or_default();
    for path in ["HEAD", "index", "packed-refs", reference.trim()] {
        if path.is_empty() { continue; }
        if let Ok(output) = std::process::Command::new("git").args(["rev-parse", "--git-path", path]).output() {
            if output.status.success() { println!("cargo:rerun-if-changed={}", String::from_utf8_lossy(&output.stdout).trim()); }
        }
    }
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=build.rs");
    // oboe (rodio's Android audio backend) compiles C++, but nothing in the dependency graph
    // links the C++ runtime — the Android linker happily defers the unresolved symbols to
    // dlopen, which then aborts app startup on-device (`cannot locate symbol
    // "__cxa_pure_virtual"`). Link libc++ statically; trailing link-args land after every
    // archive on the link line, so single-pass symbol resolution sees them last.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!("cargo:rustc-link-arg=-lc++_static");
        println!("cargo:rustc-link-arg=-lc++abi");
    }

    // Windows: embed the app icon (so Explorer, the taskbar and the installer show a logo instead
    // of the generic exe glyph) plus the version block Properties -> Details reads. The .ico is
    // checked in; regenerate with `hookecho --headless-ico packaging/windows/icon.ico`.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=../../packaging/windows/icon.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../packaging/windows/icon.ico")
            .set("ProductName", "HookEcho")
            .set("FileDescription", "HookEcho — NEXRAD weather radar viewer")
            .set("LegalCopyright", "GPL-3.0-only licensed");
        if let Err(e) = res.compile() {
            println!("cargo:warning=windows resource compile failed: {e}");
        }
    }
}
