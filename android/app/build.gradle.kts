plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.sgbuscommute"
    compileSdk = 36

    defaultConfig {
        applicationId = "com.sgbuscommute"
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
    // Kotlin glue: NotificationCompat. 1.17.0+ adds the Android 16 Live Update
    // promotion APIs (setRequestPromotedOngoing / setShortCriticalText).
    implementation("androidx.core:core-ktx:1.17.0")
}
