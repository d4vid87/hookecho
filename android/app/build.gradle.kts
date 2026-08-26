plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

dependencies {
    // GameActivity: the Java half of the activity the Rust `android_main` runs inside (the native
    // half is compiled by the `android-activity` crate). It also carries GameTextInput, which is
    // what gives the app the real system IME instead of NativeActivity's raw ASCII key events.
    // Pulls androidx.appcompat transitively — hence the AppCompat theme in res/values/themes.xml,
    // without which the activity throws at startup.
    implementation("androidx.games:games-activity:3.0.5")
    // Named explicitly rather than left to games-activity: the AAR marks its appcompat dependency
    // compile-only, so without this the Theme.AppCompat parent style doesn't resolve at all.
    implementation("androidx.appcompat:appcompat:1.7.0")
    // Survives process death and reboot: the alert foreground service can be killed, and only a
    // scheduled worker gets the poll going again (see AlertWorker.kt / BootReceiver.kt).
    implementation("androidx.work:work-runtime-ktx:2.9.1")
}

// Single source of truth for the version: the workspace Cargo.toml. Hand-maintained gradle
// versions drifted (0.4.0 here vs 0.5.0 there) — 0.5.0 -> versionCode 50000, which also jumps
// one-way above every previously published code.
val cargoVersion: String =
    Regex("""(?m)^version\s*=\s*"([^"]+)"""")
        .find(rootProject.file("../Cargo.toml").readText())
        ?.groupValues?.get(1)
        ?: error("no version in ../Cargo.toml")
// major.minor.patch -> MmmmmmPPPP, monotonic: 0.5.0 -> 50000, 1.2.3 -> 1020003.
val cargoVersionCode: Int = cargoVersion.split(".", "-")
    .let { it[0].toInt() * 1_000_000 + it[1].toInt() * 10_000 + it[2].toInt() }

android {
    namespace = "io.hookecho.HookEcho"
    compileSdk = 35

    defaultConfig {
        applicationId = "io.hookecho.HookEcho"
        // API 29 (Android 10): guarantees Vulkan 1.1 + AAudio + scoped storage semantics we design
        // around. arm64-v8a only for v1 — every phone that can run this ships it.
        minSdk = 29
        targetSdk = 35
        versionCode = cargoVersionCode
        versionName = cargoVersion
        ndk {
            abiFilters += "arm64-v8a"
        }
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    // cargo-ndk drops libhookecho.so into src/main/jniLibs/<abi>/; AGP just packages the prebuilt
    // library — the Rust build is driven outside Gradle (see ../build.sh and the release workflow).
    sourceSets["main"].jniLibs.srcDirs("src/main/jniLibs")

    // cargo-ndk copies whatever it finds next to the cdylib, including build-script artifacts
    // (stale `libmvt_reader-*.so` orphans were shipping in every APK). The .so dir is a build
    // output, not a source tree — anything but libhookecho.so is garbage.
    tasks.named("preBuild") {
        doFirst {
            file("src/main/jniLibs").listFiles()?.forEach { abi ->
                abi.listFiles()?.filter { it.name != "libhookecho.so" }?.forEach { it.delete() }
            }
        }
    }

    buildTypes {
        // Debug is signed with the default debug key → directly `adb install`-able for sideload.
        // Release is signed in CI from a keystore in repo secrets (see .github/workflows/release.yml).
        release {
            isMinifyEnabled = false
        }
    }

    // Stable APK signature across CI runs: without this, every workflow run generates a fresh
    // debug keystore and phones refuse the update (INSTALL_FAILED_UPDATE_INCOMPATIBLE). CI
    // materializes the keystore from repo secrets and points these env vars at it; local builds
    // without the vars keep the default debug signing.
    val ksFile = System.getenv("HOOKECHO_KEYSTORE").takeUnless { it.isNullOrEmpty() }
    val ksPass = System.getenv("HOOKECHO_KEYSTORE_PASS").takeUnless { it.isNullOrEmpty() }
    if (ksFile != null && ksPass != null) {
        signingConfigs {
            create("stable") {
                storeFile = file(ksFile)
                storePassword = ksPass
                keyAlias = "hookecho"
                keyPassword = ksPass
                storeType = "PKCS12"
            }
        }
        buildTypes.getByName("debug").signingConfig = signingConfigs.getByName("stable")
        buildTypes.getByName("release").signingConfig = signingConfigs.getByName("stable")
    }
}
