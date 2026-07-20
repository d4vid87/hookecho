// Root Gradle build. The single `:app` module packages the Rust `.so` (built by cargo-ndk) into a
// NativeActivity APK — there is no Java/Kotlin source.
plugins {
    id("com.android.application") version "8.5.2" apply false
}
