# InkKeep Development Guide

English | [繁體中文](development.zh-TW.md)

## Building

You need [Rust](https://rustup.rs/) (MSVC toolchain), [Node.js](https://nodejs.org/) and pnpm.

```powershell
git clone https://github.com/John8805/InkKeep.git inkkeep
cd inkkeep
pnpm --dir ui install
pnpm --dir ui build
cd crates\inkkeep-app
..\..\ui\node_modules\.bin\tauri build --no-bundle
```

The executable is `target/release/inkkeep.exe`, a single file with the frontend built in.

**Build with the Tauri CLI; `cargo build --release` alone is not enough.** `tauri-build` only knows it is a release build
when run through the CLI. A plain cargo build keeps `cfg(dev)`, and the resulting executable still tries to load `localhost:5173`,
showing a "can't reach this page" error.

`--no-bundle` skips the installer; the project currently ships a single executable.

For development, run the frontend dev server:

```powershell
pnpm --dir ui dev          # in a separate terminal
cargo run -p inkkeep-app   # the debug build connects to localhost:5173
```

## Project layout

```
crates/
├── inkkeep-core/   Data model, KDBX access, search, templates, password generator, bookmark and password import
├── inkkeep-win/    Windows integration: foreground window, clipboard, key injection, Credential Manager
├── inkkeep-cli/    Developer CLI for exercising core without the UI
└── inkkeep-app/    Tauri app
ui/
├── src/locales/    UI strings, one file per language
└── src/            Svelte 5 frontend
```

`inkkeep-core` does not touch the OS or UI, so its full test suite runs on a headless CI machine.

## Tests

```powershell
cargo test
cargo clippy --all-targets
```

`inkkeep-core/tests/vault.rs` creates real KDBX files. Every open and save runs Argon2id (64 MiB), and with two-tier keys one `open_with_password` runs it twice, so these tests are much slower than the unit tests.

The tests in `inkkeep-win/src/credstore.rs` write to the current user's Credential Manager, using fake paths with random suffixes, and each test cleans up after itself.

## Developer CLI

```powershell
$env:INKKEEP_PASSWORD = "your master password"
cargo run -p inkkeep-cli -- --vault path\to\vault.kdbx list
cargo run -p inkkeep-cli -- --vault path\to\vault.kdbx vault-key   # key for KeePassXC
cargo run -p inkkeep-cli -- --vault path\to\vault.kdbx render <id prefix> --input "client=Mr. Wang"
cargo run -p inkkeep-cli -- import-preview --source chrome   # read only
cargo run -p inkkeep-cli -- gen --length 24 --count 3
```

## Checking the platform layer by hand

Most of `inkkeep-win`'s injection behavior needs a real window:

```powershell
cargo run -p inkkeep-win --example probe -- verify-paste "test text"
cargo run -p inkkeep-win --example probe -- verify-type "test text"
cargo run -p inkkeep-win --example probe -- verify-interrupt
```

## Adding a language

1. Copy `ui/src/locales/en.js`, translate it, and save it as `<language code>.js`.
2. Add a line for it to both `BUNDLES` and `LANGUAGES` in `ui/src/i18n.svelte.js`.
3. Add an entry to both `SUPPORTED` and `tray()` in `crates/inkkeep-app/src/i18n.rs` (the tray menu).

`en.js` is the fallback; missing keys fall back to English.
