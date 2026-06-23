plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.serpoul.sgbusready"
    compileSdk = 36

    defaultConfig {
        applicationId = "com.serpoul.sgbusready"
        minSdk = 24
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters += listOf("arm64-v8a") }
    }
    buildTypes {
        getByName("debug") { isMinifyEnabled = false }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    // The Rust cdylib is pre-built into src/main/jniLibs by cargo-ndk; AGP only
    // packages it, so no NDK / externalNativeBuild config is needed here.
}

dependencies {
    // Phase B: Kotlin glue + hasCode=true. core-ktx provides NotificationCompat.
    implementation("androidx.core:core-ktx:1.13.1")
}
