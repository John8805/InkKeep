//! 應用程式狀態：保險庫、第二層金鑰、搜尋索引、待送出的目標視窗。
//!
//! 兩層各自獨立開關：
//!
//! - **保險庫**用第一層金鑰開，金鑰存在 Windows 認證管理員，所以正常情況下
//!   程式一啟動就是開著的，片語與書籤立刻能搜。
//! - **第二層金鑰**只在使用者為了用密碼而輸入主密碼之後才存在，閒置逾時就清掉。
//!   清它不會連帶關掉保險庫。
//!
//! 兩者都是整個從記憶體移除。

use inkkeep_core::crypto::SecretKey;
use inkkeep_core::model::Item;
use inkkeep_core::search::Index;
use inkkeep_core::vault::Vault;
use inkkeep_win::FocusToken;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

pub struct AppState {
    inner: Mutex<Inner>,
}

struct Inner {
    vault_path: PathBuf,
    /// `None` 表示保險庫沒開——沒有快取金鑰，或金鑰過期了。
    open: Option<OpenVault>,
    /// 第二層金鑰。`None` 表示密碼項目仍是密文。
    secret: Option<SecretKey>,
    /// 按下快捷鍵時擷取的前景視窗。
    pending_target: Option<FocusToken>,
    last_activity: Instant,
    /// 閒置幾分鐘清掉第二層金鑰，0 表示不自動清。
    lock_after_minutes: u32,
    /// 註冊失敗的快捷鍵字串；`None` 表示註冊成功。
    hotkey_conflict: Option<String>,
}

struct OpenVault {
    vault: Vault,
    index: Index,
}

impl OpenVault {
    fn reindex(&mut self) {
        self.index = Index::build(self.vault.items());
    }
}

impl AppState {
    pub fn new(vault_path: PathBuf, lock_after_minutes: u32) -> Self {
        AppState {
            inner: Mutex::new(Inner {
                vault_path,
                open: None,
                secret: None,
                pending_target: None,
                last_activity: Instant::now(),
                lock_after_minutes,
                hotkey_conflict: None,
            }),
        }
    }

    pub fn vault_path(&self) -> PathBuf {
        self.inner.lock().expect("state 毒化").vault_path.clone()
    }

    pub fn set_vault_path(&self, path: PathBuf) {
        self.inner.lock().expect("state 毒化").vault_path = path;
    }

    // ---------- 第一層：保險庫 ----------

    pub fn is_locked(&self) -> bool {
        self.inner.lock().expect("state 毒化").open.is_none()
    }

    pub fn unlock(&self, vault: Vault) {
        let index = Index::build(vault.items());
        let mut inner = self.inner.lock().expect("state 毒化");
        inner.open = Some(OpenVault { vault, index });
        inner.last_activity = Instant::now();
    }

    /// 關掉保險庫，連第二層一起。
    pub fn close(&self) {
        let mut inner = self.inner.lock().expect("state 毒化");
        inner.open = None;
        inner.secret = None;
        inner.pending_target = None;
    }

    /// 第一層金鑰的原文。
    pub fn vault_key(&self) -> Option<String> {
        self.with_vault(|v, _| v.vault_key().to_string())
    }

    // ---------- 第二層：密碼 ----------

    pub fn secrets_unlocked(&self) -> bool {
        self.inner.lock().expect("state 毒化").secret.is_some()
    }

    pub fn unlock_secrets(&self, key: SecretKey) {
        let mut inner = self.inner.lock().expect("state 毒化");
        inner.secret = Some(key);
        inner.last_activity = Instant::now();
    }

    /// 清掉第二層金鑰。保險庫照常開著——片語與書籤不受影響。
    pub fn lock_secrets(&self) {
        let mut inner = self.inner.lock().expect("state 毒化");
        inner.secret = None;
        inner.pending_target = None;
    }

    /// 取一份第二層金鑰的副本，好在不持有鎖的情況下加解密。
    /// 副本離開作用域時會清零（`SecretKey` 是 `ZeroizeOnDrop`）。
    pub fn secret(&self) -> Option<SecretKey> {
        self.inner.lock().expect("state 毒化").secret.clone()
    }

    // ---------- 閒置與目標視窗 ----------

    pub fn touch_activity(&self) {
        self.inner.lock().expect("state 毒化").last_activity = Instant::now();
    }

    pub fn idle_secs(&self) -> u64 {
        self.inner
            .lock()
            .expect("state 毒化")
            .last_activity
            .elapsed()
            .as_secs()
    }

    pub fn lock_after_minutes(&self) -> u32 {
        self.inner.lock().expect("state 毒化").lock_after_minutes
    }

    pub fn set_lock_after_minutes(&self, minutes: u32) {
        self.inner.lock().expect("state 毒化").lock_after_minutes = minutes;
    }

    pub fn hotkey_conflict(&self) -> Option<String> {
        self.inner.lock().expect("state 毒化").hotkey_conflict.clone()
    }

    pub fn set_hotkey_conflict(&self, combo: Option<String>) {
        self.inner.lock().expect("state 毒化").hotkey_conflict = combo;
    }

    pub fn set_target(&self, token: Option<FocusToken>) {
        self.inner.lock().expect("state 毒化").pending_target = token;
    }

    pub fn take_target(&self) -> Option<FocusToken> {
        self.inner.lock().expect("state 毒化").pending_target.take()
    }

    // ---------- 保險庫存取 ----------

    /// 對已開啟的保險庫做唯讀操作。沒開時回 `None`。
    pub fn with_vault<T>(&self, f: impl FnOnce(&Vault, &Index) -> T) -> Option<T> {
        let inner = self.inner.lock().expect("state 毒化");
        inner.open.as_ref().map(|o| f(&o.vault, &o.index))
    }

    /// 對已開啟的保險庫做寫入操作，完成後重建索引。
    pub fn with_vault_mut<T>(&self, f: impl FnOnce(&mut Vault) -> T) -> Option<T> {
        let mut inner = self.inner.lock().expect("state 毒化");
        let open = inner.open.as_mut()?;
        let out = f(&mut open.vault);
        open.reindex();
        Some(out)
    }

    /// 全部項目。密碼項目的 `body` 是密文。
    pub fn items(&self) -> Vec<Item> {
        self.with_vault(|v, _| v.items().to_vec())
            .unwrap_or_default()
    }
}
