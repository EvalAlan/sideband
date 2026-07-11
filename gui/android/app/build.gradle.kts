import java.io.FileInputStream
import java.util.Properties

plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

// Standard key.properties pattern: if android/key.properties exists (never committed --
// see .gitignore), load real release signing credentials from it. Otherwise fall back to
// the debug key so `flutter run --release` / local release builds keep working without
// requiring every developer to have a production keystore.
val keystorePropertiesFile = rootProject.file("key.properties")
val keystoreProperties = Properties()
val hasReleaseSigningConfig = keystorePropertiesFile.exists()
if (hasReleaseSigningConfig) {
    keystoreProperties.load(FileInputStream(keystorePropertiesFile))
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
        // TODO: Specify your own unique Application ID
        // (https://developer.android.com/studio/build/application-id.html).
        //
        // Left as com.example.sideband_gui deliberately for now -- do NOT change this without
        // a deliberate migration plan. The applicationId is part of Android's per-app storage
        // path (filesDir), which is where the Tor identity (identity.toml), ratchet state, and
        // message history live (see MainActivity.profilePath / src/main.rs). Changing it is
        // effectively creating a new, empty app from Android's point of view: existing installs
        // would lose access to their existing Tor identity and conversation history rather than
        // being migrated. That's an intentional decision for the app owner to make (and pair
        // with an explicit data-migration/export story), not something to change incidentally
        // here.
        applicationId = "com.example.sideband_gui"
        // You can update the following values to match your application needs.
        // For more information, see: https://flutter.dev/to/review-gradle-config.
        minSdk = flutter.minSdkVersion
        targetSdk = flutter.targetSdkVersion
        versionCode = flutter.versionCode
        versionName = flutter.versionName
    }

    signingConfigs {
        if (hasReleaseSigningConfig) {
            create("release") {
                storeFile = file(keystoreProperties.getProperty("storeFile"))
                storePassword = keystoreProperties.getProperty("storePassword")
                keyAlias = keystoreProperties.getProperty("keyAlias")
                keyPassword = keystoreProperties.getProperty("keyPassword")
            }
        }
    }

    buildTypes {
        release {
            // Real release signing when android/key.properties is present; otherwise fall
            // back to the debug key so local/dev `flutter run --release` builds keep working.
            signingConfig = if (hasReleaseSigningConfig) {
                signingConfigs.getByName("release")
            } else {
                signingConfigs.getByName("debug")
            }
            // Flutter runs R8 on release builds. Keep it on (smaller APK) but apply our
            // keep rules so reflection-based plugins (mobile_scanner -> ML Kit barcode)
            // survive shrinking; without these the QR scanner fails to start in release.
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

flutter {
    source = "../.."
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
}
