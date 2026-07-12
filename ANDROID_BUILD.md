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
- **Profile dir** → Uses app-specific directory: `/data/user/0/com.evalalan.sideband/files/.sideband`
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

By default release builds fall back to the **debug** key (fine for sideloading
to your own devices, not for distribution). To sign with your own key:

**1. Generate a keystore once** (keep it OUTSIDE the repo):

```bash
keytool -genkeypair -v \
  -keystore ~/keys/sideband-release.jks \
  -alias sideband -keyalg RSA -keysize 4096 -validity 10000
```

**2. Point the build at it.** Copy the template and fill in real values:

```bash
cp gui/android/key.properties.example gui/android/key.properties
$EDITOR gui/android/key.properties
```

`key.properties` (and `*.jks` / `*.keystore` / `*.p12`) are gitignored and must
never be committed. `./build.sh android` / Gradle auto-detect the file; when it
is present the release build is signed with your key, when absent it uses the
debug key. Debug builds always use the debug key.

**3. Verify which key actually signed an APK:**

```bash
"$ANDROID_HOME"/build-tools/*/apksigner verify --print-certs \
  gui/build/app/outputs/flutter-apk/app-release.apk | grep 'certificate DN'
```

- `CN=Android Debug` → still the debug fallback (no `key.properties`, or it was ignored).
- Your own DN → signed with your release key.

### applicationId — migrated to `com.evalalan.sideband`

The `applicationId` was migrated from the Flutter-template default
`com.example.sideband_gui` to `com.evalalan.sideband` (matching the desktop app)
before public release. The id is part of Android's per-app storage path
(`filesDir`), where the Tor identity (`identity.toml`), ratchet state, and
message history live, so **any pre-migration install keeps its own separate data
under the old id** — it appears as a different app. Move that data over with
`sideband export` / `sideband import` (encrypted profile archive). Do not change
the id again without another export/import migration for existing users.

## Next Steps for Production

1. **Release Signing**: Set up `gui/android/key.properties` (see above)
2. **App Bundle**: `cd gui/android && ./gradlew bundleRelease` for Play Store submission
3. **Icon**: Replace `android/app/src/main/res/mipmap-*/ic_launcher.png` with Sideband icon
4. **Metadata**: Update Play Store listing description, screenshots, privacy policy link