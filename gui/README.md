# Sideband GUI (Flutter)

Desktop/mobile GUI scaffold is now runnable.

## Start GUI (Linux desktop)

```bash
cd /home/rocky/repos/sideband/gui
../.tools/flutter/bin/flutter run -d linux
```

If you already have Flutter in PATH:

```bash
cd /home/rocky/repos/sideband/gui
flutter run -d linux
```

## Current state

- Flutter app shell runs.
- Rust API boundary exists in `../src/app_api.rs`.
- FRB config exists in `flutter_rust_bridge.yaml`.

What is still missing:

- Generated FRB bindings
- Rust `lib.rs` + `cdylib` export for FRB
- Wiring UI to real Rust backend calls

## Notes

- Running from a headless shell without a display will fail with GTK `cannot open display`.
- Android requires Android SDK/Studio setup first.
