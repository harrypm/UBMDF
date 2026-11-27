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

    buildTypes {
        release {
            isMinifyEnabled = false
            // Debug-key signed so the APK installs directly (sideload).
            signingConfig = signingConfigs.getByName("debug")
        }
    }
}
