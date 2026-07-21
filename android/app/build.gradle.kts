plugins {
    id("com.android.application")
}

android {
    namespace = "zip.batman.hookecho"
    compileSdk = 35

    defaultConfig {
        applicationId = "zip.batman.hookecho"
        // API 29 (Android 10): guarantees Vulkan 1.1 + AAudio + scoped storage semantics we design
        // around. arm64-v8a only for v1 — every phone that can run this ships it.
        minSdk = 29
        targetSdk = 35
        versionCode = 4
        versionName = "0.4.0"
        ndk {
            abiFilters += "arm64-v8a"
        }
    }

    // cargo-ndk drops libhookecho.so into src/main/jniLibs/<abi>/; AGP just packages the prebuilt
    // library — the Rust build is driven outside Gradle (see ../build.sh and the release workflow).
    sourceSets["main"].jniLibs.srcDirs("src/main/jniLibs")

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
