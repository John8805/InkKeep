# InkKeep 開發說明

[English](development.md) | 繁體中文

## 建置

需要 [Rust](https://rustup.rs/)（MSVC toolchain）、[Node.js](https://nodejs.org/) 與 pnpm。

```powershell
git clone https://github.com/John8805/InkKeep.git inkkeep
cd inkkeep
pnpm --dir ui install
pnpm --dir ui build
cd crates\inkkeep-app
..\..\ui\node_modules\.bin\tauri build --no-bundle
```

執行檔在 `target/release/inkkeep.exe`，單一個檔，前端已經打包進去。

**要用 Tauri CLI 建置，只跑 `cargo build --release` 不夠。** `tauri-build` 只有在透過 CLI 建置時
才知道這是正式版；直接用 cargo 會留著 `cfg(dev)`，做出來的執行檔仍然去連 `localhost:5173`，
開起來是「無法連線到此頁面」。

`--no-bundle` 會跳過安裝檔，這個專案目前只發佈單一執行檔。

開發時前端跑 dev server：

```powershell
pnpm --dir ui dev          # 另開一個終端機
cargo run -p inkkeep-app   # debug build 連到 localhost:5173
```

## 專案結構

```
crates/
├── inkkeep-core/   資料模型、kdbx 存取、搜尋、模板、密碼產生、書籤與密碼匯入
├── inkkeep-win/    Windows 整合：前景視窗、剪貼簿、按鍵注入、認證管理員
├── inkkeep-cli/    開發用 CLI，不需要 UI 就能驗證 core
└── inkkeep-app/    Tauri 應用
ui/
├── src/locales/    介面字串，一種語言一個檔
└── src/            Svelte 5 前端
```

`inkkeep-core` 不碰 OS 與 UI，可以在無顯示器的 CI 上跑完整測試。

## 測試

```powershell
cargo test
cargo clippy --all-targets
```

`inkkeep-core/tests/vault.rs` 會建真的 kdbx 檔，每次開關檔都跑一次 Argon2id（64 MiB），而兩層金鑰讓一次 `open_with_password` 要跑兩輪，所以比單元測試慢得多。

`inkkeep-win/src/credstore.rs` 的測試會真的寫進目前使用者的認證管理員，用的是帶隨機字尾的假路徑，每個測試自己收尾。

## 開發用 CLI

```powershell
$env:INKKEEP_PASSWORD = "你的主密碼"
cargo run -p inkkeep-cli -- --vault path\to\vault.kdbx list
cargo run -p inkkeep-cli -- --vault path\to\vault.kdbx vault-key   # KeePassXC 用的金鑰
cargo run -p inkkeep-cli -- --vault path\to\vault.kdbx render <id前綴> --input "客戶=王先生"
cargo run -p inkkeep-cli -- import-preview --source chrome   # 只讀不寫
cargo run -p inkkeep-cli -- gen --length 24 --count 3
```

## 手動驗證平台層

`inkkeep-win` 的注入行為大多要在真實視窗上試：

```powershell
cargo run -p inkkeep-win --example probe -- verify-paste "測試文字"
cargo run -p inkkeep-win --example probe -- verify-type "測試文字"
cargo run -p inkkeep-win --example probe -- verify-interrupt
```

## 新增語言

1. 複製 `ui/src/locales/en.js` 翻完，存成 `<語言代碼>.js`。
2. 在 `ui/src/i18n.svelte.js` 的 `BUNDLES` 與 `LANGUAGES` 各加一行。
3. 在 `crates/inkkeep-app/src/i18n.rs` 的 `SUPPORTED` 與 `tray()` 各加一筆（系統匣選單）。

`en.js` 是 fallback，缺的 key 會退回英文。
