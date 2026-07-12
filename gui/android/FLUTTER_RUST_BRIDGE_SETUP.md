# flutter_rust_bridge Android Setup Notes

There is **no** `com.fzyzcjy.flutter_rust_bridge` Gradle plugin and no Maven dependency named `dev.flutter.flutter_rust_bridge:frb` to add here. Do not add either; Gradle will fail resolving them.

For a real Rust-backed Android build, use the FRB codegen CLI/package workflow:

1. Add the Dart `flutter_rust_bridge` package to `pubspec.yaml`.
2. Run `frb_codegen generate` from `gui/` using `flutter_rust_bridge.yaml`.
3. Build `libsideband.so` for Android ABIs with `cargo-ndk` or the repo's `./build.sh android` after NDK/OpenSSL are configured.
4. Copy/link the generated JNI libs under `android/app/src/main/jniLibs/<abi>/` or configure Gradle source sets for them.

Until that bridge is fully generated and wired, Android builds are Flutter-shell smoke builds only; desktop still uses the CLI subprocess backend.
