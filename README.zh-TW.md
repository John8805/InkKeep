# <img src="crates/inkkeep-app/icons/icon.png" width="40" height="40"/> InkKeep

[English](README.md) | 繁體中文

InkKeep 是給常打重複文字的人用的 Windows 桌面工具。按全域快捷鍵、搜尋、按 Enter，InkKeep 就把片語、書籤網址或密碼送進你原本在用的程式。片語可以放日期、剪貼簿內容、填空欄位這類動態佔位符。所有資料存在同一個 KeePass KDBX 檔，KeePassXC 也能開啟。InkKeep 開起來直接是搜尋框，用到密碼項目時才問主密碼。

## 快速開始

目前沒有預先建置的執行檔，請先從原始碼建置（見[建置 InkKeep](#建置-inkkeep)），再執行 `inkkeep.exe`。

第一次啟動時，設定精靈會問保險庫要放哪裡，以及主密碼。保險庫放在 OneDrive、Dropbox 這類同步資料夾，就能多台電腦共用。主密碼無法重設，請務必記住。

設定完成後 InkKeep 在系統匣執行，按 `Alt+.` 叫出搜尋視窗。快捷鍵、佔位符與各項功能的細節見[使用說明](docs/user-guide.zh-TW.md)。

## 功能

### 核心功能

* 按全域快捷鍵叫出搜尋視窗，視窗出現在目前使用中的螢幕，選好的內容送進原本在用的程式
* 片語、書籤、密碼存在同一個 KDBX 4 檔，KeePassXC 也能開啟
* 片語佔位符：日期時間（可偏移、可自訂格式）、剪貼簿、送出後的游標位置、輸入欄位、下拉選單、UUID、引用其他片語
* 密碼與帳號用 `SendInput` 逐字輸入，不經過剪貼簿
* 一組主密碼推導出兩層金鑰：開啟保險庫的金鑰快取在 Windows 認證管理員，密碼欄位另外用公鑰加密，存密碼不必輸入主密碼
* 依類別（片語、書籤、密碼）與標籤過濾，打 `#` 會跳出標籤選單
* 工作區，例如把個人和公司的項目分開
* 密碼產生器

### 進階功能

* 從 Chrome、Edge、Firefox 的每個設定檔匯入書籤
* 匯入 Chrome、Firefox、Bitwarden、1Password、KeePassXC、LastPass 等匯出的密碼 CSV
* 閒置一段時間自動鎖定密碼，片語與書籤照常可用
* 存檔時讀回逐欄比對，一致才覆蓋正本，並保留 3 份備份
* 偵測 OneDrive、iCloud 的同步衝突副本
* 介面支援繁體中文與 English，有淺色、深色主題
* 開發用命令列工具（`inkkeep-cli`）

完整變更紀錄見 [CHANGELOG](CHANGELOG.md)。\
快捷鍵與細節見[使用說明](docs/user-guide.zh-TW.md)。

## 建置 InkKeep

需要 [Rust](https://rustup.rs/)（MSVC toolchain）、[Node.js](https://nodejs.org/) 與 pnpm。

```powershell
git clone https://github.com/John8805/InkKeep.git inkkeep
cd inkkeep
pnpm --dir ui install
pnpm --dir ui build
cd crates\inkkeep-app
..\..\ui\node_modules\.bin\tauri build --no-bundle
```

執行檔在 `target/release/inkkeep.exe`。請照上面用 Tauri CLI 建置；只跑 `cargo build --release` 會得到開發版。dev server、專案結構、測試與開發用 CLI 見[開發說明](docs/development.zh-TW.md)。

## 參與開發

回報問題或提出建議，請到 GitHub 開 [issue](https://github.com/John8805/InkKeep/issues)。也歡迎送 pull request，建置與測試方式見[開發說明](docs/development.zh-TW.md)。

## 授權

InkKeep 採用 [MIT 授權](LICENSE)。
