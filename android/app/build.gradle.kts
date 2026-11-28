plugins {
    id("com.android.application")
}

android {
    namespace = "io.github.ubmdf"
    compileSdk = 35

    defaultConfig {
        applicationId = "io.github.ubmdf"
        minSdk = 26
        targetSdk = 35
        // versionCode from the crate version: 1.0.2 -> 10002
        versionName = "1.0.2"
        versionCode = "1.0.2".split(".").let { (a, b, c) -> a.toInt() * 10000 + b.toInt() * 100 + c.toInt() }
        ndk { abiFilters += listOf("arm64-v8a") }
    }

    // libubmdf_android.so is built by cargo-ndk into src/main/jniLibs/<abi>/
    sourceSets["main"].jniLibs.srcDirs("src/main/jniLibs")

    // Committed debug key (android/debug.keystore, PKCS#12): every build has
    // the same signature, so a new APK installs as an update over the old one
    // (a fresh CI-generated key each run would force uninstall first).
    // Sideload key only, not a Play Store identity.
    signingConfigs {
        create("ubmdfDebug") {
            storeFile = rootProject.file("debug.keystore")
            storeType = "pkcs12"
            storePassword = "android"
            keyAlias = "androiddebugkey"
            keyPassword = "android"
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("ubmdfDebug")
        }
    }

    // Store .so uncompressed and page-aligned (16 KB) so Android 15+ devices
    // with 16 KB pages can map the library directly.
    packaging {
        jniLibs { useLegacyPackaging = false }
    }
}
