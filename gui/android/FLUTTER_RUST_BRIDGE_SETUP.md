# flutter_rust_bridge Android Gradle Plugin
# Apply this in android/app/build.gradle.kts

# Add to plugins block:
# id("dev.flutter.flutter-gradle-plugin")
# id("com.fzyzcjy.flutter_rust_bridge") version "2.0.0" apply false

# Then in android block, add:
# flutterRustBridge {
#     // Path to the Rust crate containing the FFI functions
#     crateRoot = file("../../").absolutePath
#     // The Rust crate name (from Cargo.toml)
#     crateName = "sideband"
#     // Output directory for generated JNI/Kotlin bindings
#     outputDir = file("src/main/kotlin").absolutePath
#     // Package for generated Kotlin bindings
#     packageName = "com.example.sideband_gui.bridge"
#     // Enable debug symbols
#     debug = true
# }

# Dependencies block:
# implementation("dev.flutter.flutter_rust_bridge:frb:2.0.0")
