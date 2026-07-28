// Root Gradle build. The single `:app` module packages the Rust `.so` (built by cargo-ndk) into a
// NativeActivity APK. The only JVM source is the background alert service (see
// app/src/main/kotlin) — everything the user sees is drawn by Rust.
plugins {
    id("com.android.application") version "8.5.2" apply false
    id("org.jetbrains.kotlin.android") version "1.9.24" apply false
}
