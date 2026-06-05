# Sideband Android App — Build & Test in Android Studio

## Prerequisites (on your machine / mercury)

```bash
# 1. Install Android Studio (includes SDK + NDK)
#    https://developer.android.com/studio
#    Or via CachyOS: yay -S android-studio

# 2. Install Flutter (if not already)
#    https://flutter.dev/docs/get-started/install/linux
#    Or use vendored: ~/repos/sideband/gui/.tools/flutter/

# 3. Install Rust Android targets
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android

# 4. Set NDK path (Android Studio default)
export ANDROID_NDK_HOME=~/Android/Sdk/ndk/27.0.12077973  # adjust version
# Add to ~/.bashrc or ~/.zshrc
```

---

## One-Time Project Setup

```bash
cd ~/repos/sideband

# Build Rust JNI libraries for Android
./build-android-rust.sh
```

This compiles `libsideband.so` for all 4 Android ABIs into `target/<arch>/release/`.

---

## Open in Android Studio

1. **File → Open** → select `~/repos/sideband/gui` (the Flutter project root)
2. Android Studio will detect `android/` subproject and sync Gradle
3. Wait for Gradle sync to complete

---

## Build & Run Debug APK

### Via Android Studio UI:
1. Select `app` module in toolbar
2. Select a device/emulator (or create one: **Tools → Device Manager → Create Device**)
3. Click **Run ▶** (green play button) → builds `app/build/outputs/apk/debug/app-debug.apk`

### Via command line:
```bash
cd ~/repos/sideband/gui/android
./gradlew assembleDebug
# Output: app/build/outputs/apk/debug/app-debug.apk
```

---

## Run on Physical Device

```bash
# Enable USB debugging on phone
# Connect via USB
adb devices  # should show device

# Install & run
cd ~/repos/sideband/gui/android
./gradlew installDebug
```

---

## Key Files Modified for Android

| File | Purpose |
|------|---------|
| `Cargo.toml` | Added `[lib]` cdylib + `[[bin]]` for dual binary/lib build |
| `android/app/build.gradle.kts` | Standard Flutter Android app config; no FRB Gradle plugin/dependency |
| `android/settings.gradle.kts` | Standard Flutter Gradle plugin setup |
| `flutter_rust_bridge.yaml` | Points to `../src/app_api.rs` for future FFI generation |
| `build-android-rust.sh` | Script to compile Rust `.so` for all Android ABIs once NDK/OpenSSL are configured |

---

## Architecture Notes

- **Flutter UI** → Dart calls generated Kotlin bridge (`com.example.sideband_gui.bridge`)
- **Kotlin bridge** → JNI calls `libsideband.so` (Rust `app_api.rs` functions)
- **Rust** → Runs same `sideband` logic (contacts, messages, Tor via Arti)
- **Profile dir** → Uses app-specific directory: `/data/user/0/com.example.sideband_gui/files/.sideband`

---

## Troubleshooting

| Issue | Fix |
|-------|-----|
| `flutter.sdk not set in local.properties` | Add `flutter.sdk=/path/to/flutter` to `android/local.properties` |
| `NDK not found` | Set `ANDROID_NDK_HOME` or install NDK via SDK Manager |
| `libsideband.so not found` | Run `./build-android-rust.sh` from repo root |
| `Duplicate class` errors | Clean: `./gradlew clean` then rebuild |
| Tor bootstrap fails on Android | Arti needs network permission; add `<uses-permission android:name="android.permission.INTERNET"/>` to `AndroidManifest.xml` (already in Flutter template) |

---

## Next Steps for Production

1. **Signing**: Create `key.properties` + configure `signingConfigs.release` in `app/build.gradle.kts`
2. **App ID**: Change `applicationId` from `com.example.sideband_gui` to `com.evalan.sideband`
3. **Permissions**: Add `FOREGROUND_SERVICE` for Tor listener background operation
4. **App Bundle**: `./gradlew bundleRelease` for Play Store
5. **Icon**: Replace `android/app/src/main/res/mipmap-*/ic_launcher.png` with Sideband icon