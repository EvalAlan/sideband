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

Optional overrides:

```bash
export SIDEBAND_BIN=/absolute/path/to/sideband
export SIDEBAND_PROFILE=~/.sideband
```

## Run (Linux)

```bash
cd /path/to/sideband/gui
flutter pub get
flutter run -d linux
```

(Flutter in `PATH` is assumed; use `flutter --version` to verify it is installed.)

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

## What's implemented

- **Android**: Native Rust bridge (dart:ffi to libsideband.so), file transfers via transfers UI, group messaging/management, foreground service for background operation, message notifications, offline retry-queue banner
- **TUI**: Full command-line interface with message history, contact management, file transfer management
- **Desktop (Linux)**: Basic chat UI; transfer management still routed to TUI

## Still missing

- Desktop file transfer UI (Linux/macOS/Windows)
- Light theme (dark theme only)
- Delivery receipts (message read status)
- Ratchet/session indicators in chat header
