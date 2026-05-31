# Sideband GUI (Flutter)

This is now a working desktop chat client shell backed by the Sideband CLI.

## What works

- Loads contacts from `sideband contact list`
- Loads per-contact history from `sideband history --contact <name>`
- Sends messages via `sideband send --to <name> --message <text>`
- Auto-refreshes history every 4 seconds
- Manual refresh button for contacts + history
- Shows message timestamp/status/id metadata

## Runtime requirements

- A built Sideband binary (`../target/debug/sideband`) **or** `sideband` in `PATH`
- Existing Sideband profile + contacts (create via CLI)
- Linux desktop session for `flutter run -d linux` (not headless SSH shell)

Optional override:

```bash
export SIDEBAND_BIN=/absolute/path/to/sideband
```

## Run (Linux)

```bash
cd /home/rocky/repos/sideband/gui
../.tools/flutter/bin/flutter pub get
../.tools/flutter/bin/flutter run -d linux
```

If Flutter is in `PATH`:

```bash
cd /home/rocky/repos/sideband/gui
flutter pub get
flutter run -d linux
```

## Build prerequisites (Linux)

Arch/CachyOS:

```bash
sudo pacman -S --needed base-devel cmake ninja clang gtk3 pkgconf
```

Debian/Ubuntu:

```bash
sudo apt update
sudo apt install -y build-essential cmake ninja-build clang libgtk-3-dev pkg-config
```

## Still missing

- Native Rust bridge (flutter_rust_bridge) instead of shelling out to CLI
- File transfer workflow in GUI
- Ratchet/session indicators in chat header
- Contact add/edit flows in GUI
