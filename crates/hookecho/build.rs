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
}
