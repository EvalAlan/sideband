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
    namespace = "com.evalalan.sideband"
    compileSdk = flutter.compileSdkVersion
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        // The applicationId is part of Android's per-app storage path (filesDir),
        // where the Tor identity, ratchet state, and message history live. It was
        // migrated from the com.example.* Flutter-template default to this real id
        // before public release; existing pre-migration installs keep their own
        // (old) data and can move over with `sideband export` / `import`. Do not
        // change it again without another export/import migration story.
        applicationId = "com.evalalan.sideband"
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
            //
            // A debug-signed build must NEVER be distributed: users could not install
            // updates signed with the real key, and app stores reject it. Distribution
            // builds therefore pass -PrequireReleaseSigning=true (see build.sh release)
            // which turns the silent fallback into a hard failure.
            val requireReleaseSigning =
                (project.findProperty("requireReleaseSigning") as String?)?.toBoolean() ?: false
            if (!hasReleaseSigningConfig && requireReleaseSigning) {
                throw GradleException(
                    "Release signing required but android/key.properties is missing. " +
                        "See ANDROID_BUILD.md to generate a keystore, or drop " +
                        "-PrequireReleaseSigning for a local debug-signed build."
                )
            }
            signingConfig = if (hasReleaseSigningConfig) {
                signingConfigs.getByName("release")
            } else {
                logger.warn(
                    "WARNING: signing this release build with the DEBUG key " +
                        "(android/key.properties not found). Do not distribute this artifact."
                )
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
