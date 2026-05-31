# Sideband GUI (Flutter)

Current state: scaffold only.

## What works now
- Flutter app shell under `gui/`
- Rust API boundary scaffold exists in `src/app_api.rs`

## What does NOT work yet
- No generated FRB bindings
- No Rust `cdylib` target yet
- No desktop/mobile runner folders yet (`flutter create .` not run)

## Next commands (once Flutter is installed)

```bash
cd gui
flutter create .
flutter pub get
```

Install FRB codegen (one-time):

```bash
cargo install flutter_rust_bridge_codegen
```

Generate bridge:

```bash
cd gui
flutter_rust_bridge_codegen generate --config flutter_rust_bridge.yaml
```

Then run desktop (Linux):

```bash
flutter run -d linux
```

## Critical refactor pending
`src/app_api.rs` is in a binary crate right now. For FRB, Sideband needs a proper Rust library target (`[lib] crate-type = ["cdylib", "staticlib", "rlib"]`) with the API exported from `src/lib.rs`.
