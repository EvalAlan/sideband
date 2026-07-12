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
export ANDROID_NDK_HOME=~/Android/Sdk/ndk/28.2.13676358  # or any recent NDK version
# Add to ~/.bashrc or ~/.zshrc
```

---

## One-Time Project Setup

```bash
cd ~/repos/sideband

# Build the Rust JNI libraries + the release APK for Android
./build.sh android
```

This compiles `libsideband.so` for all 4 Android ABIs into
`gui/android/app/src/main/jniLibs/<abi>/` and then builds the release APK.
(Use `./build.sh` with no arguments to build every client: `tui`, `desktop`, `android`.)

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
| `Cargo.toml` | `[lib]` cdylib for FFI exports; `[[bin]]` for CLI binary |
| `src/app_api.rs` | Rust FFI functions (e.g., `sideband_api_*`, `sideband_api_send_group_message`, `sideband_api_list_transfers`) called by Dart |
| `gui/lib/main.dart` | Dart FFI bindings in `_MobileApi` class; calls `libsideband.so` directly |
| `gui/android/app/src/main/kotlin/MainActivity.kt` | MethodChannel handler for profilePath, openFile, foreground-service, notifications |
| `gui/android/app/src/main/kotlin/ListenerForegroundService.kt` | Foreground service to keep Tor listener running in background |
| `android/app/build.gradle.kts` | Standard Flutter Android app config |
| `android/settings.gradle.kts` | Standard Flutter Gradle plugin setup |
| `build.sh` | Unified build script (`./build.sh android` builds Rust `.so` for all ABIs + the APK; auto-detects NDK) |

---

## Architecture Notes

- **Flutter UI** → Dart calls `libsideband.so` directly via `dart:ffi` (see `gui/lib/main.dart` `_MobileApi`)
- **Kotlin helpers** → MethodChannel ("sideband/native") provides profilePath, openFile, foreground-service, and notification support
- **Rust** → Runs same `sideband` logic (contacts, messages, Tor via Arti)
- **Profile dir** → Uses app-specific directory: `/data/user/0/com.example.sideband_gui/files/.sideband`
- **Foreground Service** → Keeps Tor listener active in background (see `ListenerForegroundService.kt`)

---

## Troubleshooting

| Issue | Fix |
|-------|-----|
| `flutter.sdk not set in local.properties` | Add `flutter.sdk=/path/to/flutter` to `android/local.properties` |
| `NDK not found` | Set `ANDROID_NDK_HOME` or install NDK via SDK Manager |
| `libsideband.so not found` | Run `./build.sh android` from repo root |
| `Duplicate class` errors | Clean: `./gradlew clean` then rebuild |
| Tor bootstrap fails on Android | Arti needs network permission; add `<uses-permission android:name="android.permission.INTERNET"/>` to `AndroidManifest.xml` (already in Flutter template) |

---

## Release Signing

To sign the APK for release, create `gui/android/key.properties` with standard Flutter format:

```properties
storeFile=../path/to/your/keystore.jks
storePassword=your_store_password
keyAlias=your_key_alias
keyPassword=your_key_password
```

`./build.sh android` and the Gradle build will automatically detect this file. If absent, builds default to debug signing.

**Note**: The `applicationId` remains `com.example.sideband_gui` by design; changing it will orphan existing user data on devices that have the app installed. Use Play Store internal testing tracks to validate signed builds without changing the ID.

## Next Steps for Production

1. **Release Signing**: Set up `gui/android/key.properties` (see above)
2. **App Bundle**: `cd gui/android && ./gradlew bundleRelease` for Play Store submission
3. **Icon**: Replace `android/app/src/main/res/mipmap-*/ic_launcher.png` with Sideband icon
4. **Metadata**: Update Play Store listing description, screenshots, privacy policy link