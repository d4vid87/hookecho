fn main() {
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
            .set("ProductName", "Hook Echo-WX")
            .set(
                "FileDescription",
                "Hook Echo-WX — NEXRAD weather radar viewer",
            )
            .set("LegalCopyright", "MIT licensed");
        if let Err(e) = res.compile() {
            println!("cargo:warning=windows resource compile failed: {e}");
        }
    }
}
