plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
    id("com.fzyzcjy.flutter_rust_bridge") version "2.0.0"
}

android {
    namespace = "com.example.sideband_gui"
    compileSdk = flutter.compileSdkVersion
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        // TODO: Specify your own unique Application ID (https://developer.android.com/studio/build/application-id.html).
        applicationId = "com.example.sideband_gui"
        // You can update the following values to match your application needs.
        // For more information, see: https://flutter.dev/to/review-gradle-config.
        minSdk = flutter.minSdkVersion
        targetSdk = flutter.targetSdkVersion
        versionCode = flutter.versionCode
        versionName = flutter.versionName
    }

    buildTypes {
        release {
            // TODO: Add your own signing config for the release build.
            // Signing with the debug keys for now, so `flutter run --release` works.
            signingConfig = signingConfigs.getByName("debug")
        }
    }
}

flutterRustBridge {
    // Path to the Rust crate containing the FFI functions
    crateRoot = file("../../").absolutePath
    // The Rust crate name (from Cargo.toml)
    crateName = "sideband"
    // Output directory for generated JNI/Kotlin bindings
    outputDir = file("src/main/kotlin").absolutePath
    // Package for generated Kotlin bindings
    packageName = "com.example.sideband_gui.bridge"
    // Enable debug symbols
    debug = true
}

dependencies {
    implementation("dev.flutter.flutter_rust_bridge:frb:2.0.0")
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

flutter {
    source = "../.."
}
