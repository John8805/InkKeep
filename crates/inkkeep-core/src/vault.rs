//! kdbx 保險庫讀寫。
//!
//! 寫入流程：寫暫存檔 → **重新讀回逐欄比對** → 備份輪替 → 才覆蓋正本。
//! 比對不一致就回錯誤，正本維持原樣。
//!
//! 檔案用第一層金鑰加密（見 [`crate::crypto`]），開起來之後片語與書籤就是明文。
//! 密碼項目的 body 在這一層仍是密文，要第二層金鑰才解得開——這個模組只負責
//! 原樣搬運。

use crate::crypto::{self, CryptoError, SecretKey};
use crate::model::{Item, ItemKind};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use keepass::config::{DatabaseConfig, KdfConfig};
use keepass::db::{fields, CustomDataItem, CustomDataValue, Database, GroupId};
use keepass::DatabaseKey;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use zeroize::Zeroizing;

/// 保留幾份備份。
pub const BACKUP_COUNT: usize = 3;

/// 新建保險庫的 KDF 參數。
const ARGON2_MEMORY_BYTES: u64 = 64 * 1024 * 1024;
const ARGON2_ITERATIONS: u64 = 3;
const ARGON2_PARALLELISM: u32 = 4;

/// 第二層金鑰的材料，放在 kdbx 的 CustomData。
///
/// 這三個鍵名已經寫進使用者的保險庫，**永遠不能改**，也不跟著產品名稱走
/// （見 `crypto` 模組的「檔案格式識別字」）。
const CD_SECRET_SALT: &str = "snipkit.secret.salt";
const CD_SECRET_VERIFIER: &str = "snipkit.secret.verifier";

/// 第二層金鑰對應的 X25519 公鑰。只需第一層金鑰就讀得到。
const CD_PUBLIC_KEY: &str = "snipkit.secret.pubkey";

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("wrong password")]
    WrongPassword,
    #[error("vault file not found: {0}")]
    NotFound(PathBuf),
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("kdbx error: {0}")]
    Kdbx(String),
    /// 寫入後重新讀回，內容與記憶體不一致。正本未被覆蓋。
    ///
    /// `detail` 指出第一個對不上的欄位。
    #[error("write verification failed ({detail}); original left untouched, candidate at {candidate}")]
    VerifyFailed { candidate: PathBuf, detail: String },
    /// 保險庫裡沒有第二層金鑰的材料，只在 CustomData 被外部改動時出現。
    #[error("vault has no second-tier key material")]
    NoSecretKeyMaterial,
    #[error("crypto error: {0}")]
    Crypto(#[from] CryptoError),
    #[error("workspace not found")]
    WorkspaceNotFound,
    #[error("workspace still has {0} items")]
    WorkspaceNotEmpty(usize),
    #[error("cannot delete the last workspace")]
    LastWorkspace,
    /// `empty`、`too-long`、`duplicate`
    #[error("invalid workspace name: {0}")]
    WorkspaceName(&'static str),
}

/// 工作區名稱上限（字元數）。
pub const MAX_WORKSPACE_NAME_LEN: usize = 50;

/// 一個工作區就是根群組底下的一個 kdbx 群組。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Workspace {
    pub id: Uuid,
    pub name: String,
}

impl VaultError {
    fn io(path: &Path, source: std::io::Error) -> Self {
        VaultError::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

pub struct Vault {
    path: PathBuf,
    /// 第一層金鑰的原文。
    vault_key: Zeroizing<String>,
    key: DatabaseKey,
    /// 保留原始 kdbx，存檔時在它上面改，不從零重建——
    /// 這樣 crate 有解析但我們不使用的欄位（附件、自訂圖示等）得以保留。
    db: Database,
    items: Vec<Item>,
}

impl Vault {
    /// 建立新保險庫並立刻寫檔。兩層金鑰都在這裡定案。
    pub fn create(path: &Path, password: &str) -> Result<Vault, VaultError> {
        let mut db = Database::new();
        let mut config = DatabaseConfig::default();
        config.kdf_config = KdfConfig::Argon2id {
            iterations: ARGON2_ITERATIONS,
            memory: ARGON2_MEMORY_BYTES,
            parallelism: ARGON2_PARALLELISM,
            version: argon2::Version::Version13,
        };
        db.config = config;
        db.meta.database_name = Some("InkKeep".to_string());

        let salt = crypto::random_salt();
        let secret = crypto::derive_secret_key(password, &salt)?;
        write_secret_material(&mut db, &salt, &secret)?;

        let vault_key = Zeroizing::new(crypto::derive_vault_key(password)?);

        let vault = Vault {
            path: path.to_path_buf(),
            key: DatabaseKey::new().with_password(&vault_key),
            vault_key,
            db,
            items: Vec::new(),
        };
        vault.write_to(path)?;
        Ok(vault)
    }

    /// 用第一層金鑰開檔。
    pub fn open(path: &Path, vault_key: &str) -> Result<Vault, VaultError> {
        let key = DatabaseKey::new().with_password(vault_key);
        let (db, items) = read_at(path, key.clone())?;
        Ok(Vault {
            path: path.to_path_buf(),
            vault_key: Zeroizing::new(vault_key.to_string()),
            key,
            db,
            items,
        })
    }

    /// 用主密碼開檔。
    ///
    /// 遇到舊格式保險庫（kdbx 的密碼就是主密碼本身、密碼欄位是明文）
    /// 會就地升級成兩層並存檔。
    pub fn open_with_password(path: &Path, password: &str) -> Result<Vault, VaultError> {
        let vault_key = crypto::derive_vault_key(password)?;
        match Vault::open(path, &vault_key) {
            Ok(vault) => Ok(vault),
            Err(VaultError::WrongPassword) => {
                let mut legacy = Vault::open(path, password)?;
                legacy.upgrade_to_two_tier(password)?;
                Ok(legacy)
            }
            Err(other) => Err(other),
        }
    }

    /// 第一層金鑰的原文。
    pub fn vault_key(&self) -> &str {
        &self.vault_key
    }

    /// 推導第二層金鑰並驗證主密碼。
    pub fn unlock_secrets(&self, password: &str) -> Result<SecretKey, VaultError> {
        let (salt, verifier) = self.secret_material()?;
        let key = crypto::derive_secret_key(password, &salt)?;
        if crypto::check_verifier(&key, &verifier) {
            Ok(key)
        } else {
            Err(VaultError::WrongPassword)
        }
    }

    fn secret_material(&self) -> Result<(Vec<u8>, String), VaultError> {
        read_secret_material(&self.db).ok_or(VaultError::NoSecretKeyMaterial)
    }

    /// 這個保險庫的公鑰。
    pub fn public_key(&self) -> Option<[u8; crypto::PUBLIC_KEY_LEN]> {
        let raw = get_custom_binary(&self.db, CD_PUBLIC_KEY)?;
        raw.try_into().ok()
    }

    /// 公鑰還沒寫進去就補上並存檔，回傳有沒有真的動到檔案。
    pub fn ensure_public_key(&mut self, secret: &SecretKey) -> Result<bool, VaultError> {
        if self.public_key().is_some() {
            return Ok(false);
        }
        set_custom_binary(&mut self.db, CD_PUBLIC_KEY, &crypto::public_key(secret));
        self.save()?;
        Ok(true)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 全部項目。密碼項目的 body 是密文，要 `crypto::decrypt` 才看得到內容。
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn get(&self, id: Uuid) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    // ---------- 工作區 ----------

    /// 根群組底下的群組，依檔案裡的順序。
    pub fn workspaces(&self) -> Vec<Workspace> {
        let bin = self.db.meta.recyclebin_uuid;
        self.db
            .root()
            .groups()
            .filter(|g| Some(g.id().uuid()) != bin)
            .map(|g| Workspace {
                id: g.id().uuid(),
                name: g.name.clone(),
            })
            .collect()
    }

    /// 確保至少有一個工作區（沒有就用 `default_name` 建一個），並把還沒分配的項目
    /// 收進第一個。有改動就存檔，回傳有沒有改動。
    pub fn ensure_workspaces(&mut self, default_name: &str) -> Result<bool, VaultError> {
        let mut changed = false;
        if self.workspaces().is_empty() {
            let mut root = self.db.root_mut();
            let mut group = root.add_group();
            group.name = default_name.to_string();
            changed = true;
        }
        let first = self.workspaces()[0].id;
        for item in &mut self.items {
            if item.workspace.is_none() {
                item.workspace = Some(first);
                changed = true;
            }
        }
        if changed {
            self.save()?;
        }
        Ok(changed)
    }

    pub fn add_workspace(&mut self, name: &str) -> Result<Uuid, VaultError> {
        let name = self.check_workspace_name(name, None)?;
        let mut root = self.db.root_mut();
        let mut group = root.add_group();
        group.name = name;
        let id = group.id().uuid();
        self.save()?;
        Ok(id)
    }

    pub fn rename_workspace(&mut self, id: Uuid, name: &str) -> Result<(), VaultError> {
        let name = self.check_workspace_name(name, Some(id))?;
        let mut group = self
            .workspace_group_mut(id)
            .ok_or(VaultError::WorkspaceNotFound)?;
        group.name = name;
        self.save()
    }

    /// 只刪空的工作區。**`keepass` 刪群組會連裡面的項目一起刪**，所以這個檢查不能省。
    pub fn delete_workspace(&mut self, id: Uuid) -> Result<(), VaultError> {
        if !self.workspaces().iter().any(|w| w.id == id) {
            return Err(VaultError::WorkspaceNotFound);
        }
        let count = self.items.iter().filter(|i| i.workspace == Some(id)).count();
        if count > 0 {
            return Err(VaultError::WorkspaceNotEmpty(count));
        }
        if self.workspaces().len() <= 1 {
            return Err(VaultError::LastWorkspace);
        }
        let group = self
            .db
            .group_mut(GroupId::from(id))
            .ok_or(VaultError::WorkspaceNotFound)?;
        group.remove();
        self.save()
    }

    fn workspace_group_mut(&mut self, id: Uuid) -> Option<keepass::db::GroupMut<'_>> {
        if !self.workspaces().iter().any(|w| w.id == id) {
            return None;
        }
        self.db.group_mut(GroupId::from(id))
    }

    fn check_workspace_name(&self, name: &str, except: Option<Uuid>) -> Result<String, VaultError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(VaultError::WorkspaceName("empty"));
        }
        if name.chars().count() > MAX_WORKSPACE_NAME_LEN {
            return Err(VaultError::WorkspaceName("too-long"));
        }
        let taken = self
            .workspaces()
            .iter()
            .any(|w| Some(w.id) != except && w.name.to_lowercase() == name.to_lowercase());
        if taken {
            return Err(VaultError::WorkspaceName("duplicate"));
        }
        Ok(name.to_string())
    }

    /// 新增或覆蓋一筆，然後存檔。
    pub fn upsert(&mut self, item: Item) -> Result<(), VaultError> {
        let item = normalize(item);
        match self.items.iter_mut().find(|i| i.id == item.id) {
            Some(slot) => *slot = item,
            None => self.items.push(item),
        }
        self.save()
    }

    pub fn delete(&mut self, id: Uuid) -> Result<(), VaultError> {
        self.items.retain(|i| i.id != id);
        self.save()
    }

    /// 一次新增或覆蓋多筆，只存檔一次。
    pub fn upsert_many(&mut self, items: Vec<Item>) -> Result<(), VaultError> {
        for item in items {
            let item = normalize(item);
            match self.items.iter_mut().find(|i| i.id == item.id) {
                Some(slot) => *slot = item,
                None => self.items.push(item),
            }
        }
        self.save()
    }

    /// 換一把主密碼：兩層金鑰一起換。
    ///
    /// `old_secret` 是用舊密碼解出來的第二層金鑰，呼叫前須已驗證。
    /// 密碼項目的 body 會改用新金鑰重新加密；還是明文的（外部工具直接寫進來的）一併加密。
    ///
    /// 全有或全無：在副本上換完並存檔成功才替換 `self`。任何一步失敗，
    /// 記憶體與檔案都維持舊密碼。
    pub fn change_password(
        &mut self,
        old_secret: &SecretKey,
        new_password: &str,
    ) -> Result<(), VaultError> {
        let salt = crypto::random_salt();
        let new_secret = crypto::derive_secret_key(new_password, &salt)?;

        let new_public = crypto::public_key(&new_secret);
        let mut items = self.items.clone();
        for item in &mut items {
            if item.kind == ItemKind::Password && !item.body.is_empty() {
                let plain = if crypto::is_encrypted(&item.body) {
                    Zeroizing::new(crypto::unseal(old_secret, &item.body)?)
                } else {
                    Zeroizing::new(item.body.clone())
                };
                item.body = crypto::seal(&new_public, &plain)?;
            }
        }
        let mut db = self.db.clone();
        write_secret_material(&mut db, &salt, &new_secret)?;

        let vault_key = Zeroizing::new(crypto::derive_vault_key(new_password)?);
        let mut staged = Vault {
            path: self.path.clone(),
            key: DatabaseKey::new().with_password(&vault_key),
            vault_key,
            db,
            items,
        };
        staged.save()?;
        *self = staged;
        Ok(())
    }

    /// 把舊格式（kdbx 密碼是主密碼本身、密碼欄位是明文）升級成兩層金鑰並存檔。
    fn upgrade_to_two_tier(&mut self, password: &str) -> Result<(), VaultError> {
        let salt = crypto::random_salt();
        let secret = crypto::derive_secret_key(password, &salt)?;

        let public = crypto::public_key(&secret);
        for item in &mut self.items {
            if item.kind == ItemKind::Password
                && !item.body.is_empty()
                && !crypto::is_encrypted(&item.body)
            {
                item.body = crypto::seal(&public, &item.body)?;
            }
        }
        write_secret_material(&mut self.db, &salt, &secret)?;

        self.vault_key = Zeroizing::new(crypto::derive_vault_key(password)?);
        self.key = DatabaseKey::new().with_password(&self.vault_key);
        self.save()
    }

    /// 記一次使用並存檔。
    pub fn touch(&mut self, id: Uuid) -> Result<(), VaultError> {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            item.use_count += 1;
            item.last_used_at = Some(Utc::now());
        }
        self.save()
    }

    /// 完整寫入流程，見模組說明。
    pub fn save(&mut self) -> Result<(), VaultError> {
        let candidate = with_suffix(&self.path, "tmp");
        self.write_to(&candidate)?;

        let (_, reread) = read_at(&candidate, self.key.clone())?;
        if let Some(detail) = first_difference(&self.items, &reread) {
            return Err(VaultError::VerifyFailed { candidate, detail });
        }

        rotate_backups(&self.path)?;
        std::fs::rename(&candidate, &self.path).map_err(|e| VaultError::io(&self.path, e))?;
        Ok(())
    }

    fn write_to(&self, path: &Path) -> Result<(), VaultError> {
        let mut db = self.db.clone();
        sync_into(&mut db, &self.items);
        let mut file = std::fs::File::create(path).map_err(|e| VaultError::io(path, e))?;
        db.save(&mut file, self.key.clone())
            .map_err(|e| VaultError::Kdbx(e.to_string()))?;
        Ok(())
    }
}

/// 把只對某些 kind 有意義的欄位清乾淨。
///
/// `url` 只有 Password 用得到，`apply` 也只在 Password 時寫它。記憶體裡留著
/// 舊值但檔案裡是空的，存檔後的回讀驗證就會判定不一致——類型從密碼改成片語
/// 時一定會撞到。
fn normalize(mut item: Item) -> Item {
    if item.kind != ItemKind::Password {
        item.url = None;
    }
    item
}

fn read_at(path: &Path, key: DatabaseKey) -> Result<(Database, Vec<Item>), VaultError> {
    if !path.exists() {
        return Err(VaultError::NotFound(path.to_path_buf()));
    }
    let mut file = std::fs::File::open(path).map_err(|e| VaultError::io(path, e))?;
    let db = Database::open(&mut file, key).map_err(classify_open_error)?;
    let items = read_items(&db);
    Ok((db, items))
}

// ---------- CustomData ----------

fn write_secret_material(
    db: &mut Database,
    salt: &[u8],
    secret: &SecretKey,
) -> Result<(), VaultError> {
    set_custom_binary(db, CD_SECRET_SALT, salt);
    set_custom_string(db, CD_SECRET_VERIFIER, &crypto::make_verifier(secret)?);
    set_custom_binary(db, CD_PUBLIC_KEY, &crypto::public_key(secret));
    Ok(())
}

fn read_secret_material(db: &Database) -> Option<(Vec<u8>, String)> {
    let salt = get_custom_binary(db, CD_SECRET_SALT)?;
    let verifier = get_custom_string(db, CD_SECRET_VERIFIER)?;
    Some((salt, verifier))
}

fn set_custom(db: &mut Database, key: &str, value: CustomDataValue) {
    db.meta.custom_data.insert(
        key.to_string(),
        CustomDataItem {
            value: Some(value),
            last_modification_time: Some(Utc::now().naive_utc()),
        },
    );
}

fn set_custom_string(db: &mut Database, key: &str, value: &str) {
    set_custom(db, key, CustomDataValue::String(value.to_string()));
}

/// 二進位值存成 Binary，不是 base64 字串。
///
/// `keepass` 讀 CustomData 時會先試著把值 base64 解碼，解得開就回 `Binary`
/// （`format/xml_db/meta.rs` 的 `Deserialize for CustomDataValue`）。存成
/// base64 字串會在讀回來時變成 `Binary`，型別對不上。存成 `Binary` 則是
/// 寫出去 base64、讀回來解碼，來回一致。
///
/// 驗證碼不受影響——它帶著 `snipkit-enc:v1:` 前綴，冒號不在 base64 字母表裡，
/// 解碼一定失敗，所以永遠回 `String`。
fn set_custom_binary(db: &mut Database, key: &str, value: &[u8]) {
    set_custom(db, key, CustomDataValue::Binary(value.to_vec()));
}

fn get_custom_string(db: &Database, key: &str) -> Option<String> {
    match db.meta.custom_data.get(key)?.value.as_ref()? {
        CustomDataValue::String(s) => Some(s.clone()),
        CustomDataValue::Binary(_) => None,
    }
}

fn get_custom_binary(db: &Database, key: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    match db.meta.custom_data.get(key)?.value.as_ref()? {
        CustomDataValue::Binary(b) => Some(b.clone()),
        CustomDataValue::String(s) => base64::engine::general_purpose::STANDARD.decode(s).ok(),
    }
}

fn classify_open_error(e: keepass::error::DatabaseOpenError) -> VaultError {
    use keepass::error::{DatabaseKeyError, DatabaseOpenError};
    match e {
        DatabaseOpenError::Key(DatabaseKeyError::IncorrectKey) => VaultError::WrongPassword,
        other => VaultError::Kdbx(other.to_string()),
    }
}

// ---------- Item ↔ kdbx entry ----------

/// 從 kdbx 原生欄位回推 kind。
fn detect_kind(password: &str, url: &str, notes: &str) -> ItemKind {
    if !password.is_empty() {
        ItemKind::Password
    } else if !url.is_empty() && notes.is_empty() {
        ItemKind::Bookmark
    } else {
        ItemKind::Snippet
    }
}

fn read_items(db: &Database) -> Vec<Item> {
    db.iter_all_entries()
        .map(|entry| {
            let get = |f: &str| entry.get(f).unwrap_or("").to_string();
            let title = get(fields::TITLE);
            let username = get(fields::USERNAME);
            let password = get(fields::PASSWORD);
            let url = get(fields::URL);
            let notes = get(fields::NOTES);

            let kind = detect_kind(&password, &url, &notes);
            let body = match kind {
                ItemKind::Password => password,
                ItemKind::Bookmark => url.clone(),
                ItemKind::Snippet => notes,
            };

            let times = &entry.times;
            Item {
                id: entry.id().uuid(),
                title,
                kind,
                body,
                workspace: top_level_group(db, entry.parent().id()),
                username: (!username.is_empty()).then_some(username),
                // Bookmark 的網址是 body，不重複塞進 url
                url: (kind == ItemKind::Password && !url.is_empty()).then_some(url),
                tags: entry.tags.clone(),
                use_count: times.usage_count.unwrap_or(0) as u32,
                last_used_at: times.last_access.and_then(to_utc),
                created_at: times.creation.and_then(to_utc).unwrap_or_else(Utc::now),
                updated_at: times
                    .last_modification
                    .and_then(to_utc)
                    .unwrap_or_else(Utc::now),
            }
        })
        .collect()
}

/// 把 items 寫回 db：更新既有、新增缺的、刪掉多的。
fn sync_into(db: &mut Database, items: &[Item]) {
    use std::collections::HashSet;

    let wanted: HashSet<Uuid> = items.iter().map(|i| i.id).collect();
    let existing: Vec<(keepass::db::EntryId, Uuid)> = db
        .iter_all_entries()
        .map(|e| (e.id(), e.id().uuid()))
        .collect();

    for (entry_id, uuid) in &existing {
        if !wanted.contains(uuid) {
            if let Some(entry) = db.entry_mut(*entry_id) {
                entry.remove();
            }
        }
    }

    for item in items {
        // 目標群組不存在就放根群組
        let target = item
            .workspace
            .map(GroupId::from)
            .filter(|g| db.group(*g).is_some());

        let existing_id = db
            .iter_all_entries()
            .find(|e| e.id().uuid() == item.id)
            .map(|e| e.id());
        match existing_id {
            Some(id) => {
                // 工作區變了才搬；沒變就保留它在子群組裡的位置
                let current = db
                    .entry(id)
                    .and_then(|e| top_level_group(db, e.parent().id()));
                let wanted = target.map(|g| g.uuid());
                let dest = target.unwrap_or_else(|| db.root().id());
                if let Some(mut entry) = db.entry_mut(id) {
                    if current != wanted {
                        let _ = entry.move_to(dest);
                    }
                    apply(&mut entry, item);
                }
            }
            None => {
                let dest = target.unwrap_or_else(|| db.root().id());
                let mut group = db.group_mut(dest).expect("剛確認過群組存在");
                let mut entry = group
                    .add_entry_with_id(keepass::db::EntryId::from_uuid(item.id))
                    .expect("item id 應該是唯一的");
                apply(&mut entry, item);
            }
        }
    }
}

/// 某個群組屬於哪個工作區：往上走到根群組的直接子群組為止。
/// 就是根群組本身的話回 `None`。
fn top_level_group(db: &Database, group: GroupId) -> Option<Uuid> {
    let root = db.root().id();
    let mut current = group;
    loop {
        if current == root {
            return None;
        }
        let parent = db.group(current)?.parent()?.id();
        if parent == root {
            return Some(current.uuid());
        }
        current = parent;
    }
}

fn apply(entry: &mut keepass::db::EntryMut<'_>, item: &Item) {
    entry.set_unprotected(fields::TITLE, &item.title);
    entry.set_unprotected(fields::USERNAME, item.username.as_deref().unwrap_or(""));

    // 三種 kind 各自落在自己的原生欄位。
    //
    // kind 沒變時，Item 不使用的原生欄位保留原值（例如在 KeePassXC 替密碼項目
    // 加的備註），只要保留後 `detect_kind` 仍判成同一種。kind 變了就清空，
    // 否則舊內容會殘留在看不到的欄位裡，也可能讓 `detect_kind` 判錯。
    //
    // Password 可以另外帶 URL：`detect_kind` 先看 Password 欄位，所以
    // 「Password 非空 + URL 非空」仍然判成 Password，不會被誤認成書籤。
    let current = |f: &str| entry.get(f).unwrap_or("").to_string();
    let old_url = current(fields::URL);
    let old_notes = current(fields::NOTES);
    let same_kind = detect_kind(&current(fields::PASSWORD), &old_url, &old_notes) == item.kind;

    let (password, url, notes) = match item.kind {
        ItemKind::Password => (
            item.body.as_str(),
            item.url.as_deref().unwrap_or(""),
            if same_kind { old_notes.as_str() } else { "" },
        ),
        // Notes 非空會讓 detect_kind 判成片語，書籤不能帶備註
        ItemKind::Bookmark => ("", item.body.as_str(), ""),
        // body 空的片語帶著 URL 會被判成書籤
        ItemKind::Snippet => (
            "",
            if same_kind && !item.body.is_empty() { old_url.as_str() } else { "" },
            item.body.as_str(),
        ),
    };
    entry.set_protected(fields::PASSWORD, password);
    entry.set_unprotected(fields::URL, url);
    entry.set_unprotected(fields::NOTES, notes);

    entry.tags = item.tags.clone();
    entry.times.usage_count = Some(item.use_count as usize);
    entry.times.last_access = item.last_used_at.map(to_naive);
    entry.times.creation = Some(to_naive(item.created_at));
    entry.times.last_modification = Some(to_naive(item.updated_at));
}

fn to_utc(n: NaiveDateTime) -> Option<DateTime<Utc>> {
    Utc.from_local_datetime(&n).single()
}

fn to_naive(d: DateTime<Utc>) -> NaiveDateTime {
    d.naive_utc()
}

/// 比對寫入前後是否等價，回傳第一個對不上的欄位。時間只比到秒——
/// kdbx 的時間戳精度就是秒。
///
/// 訊息裡不放 body 的內容：密碼項目的 body 是密文，但片語的 body 是明文，
/// 而這段字會一路傳到 UI 與日誌。只講長度。
fn first_difference(a: &[Item], b: &[Item]) -> Option<String> {
    if a.len() != b.len() {
        return Some(format!("item count {} -> {}", a.len(), b.len()));
    }
    let mut a: Vec<&Item> = a.iter().collect();
    let mut b: Vec<&Item> = b.iter().collect();
    a.sort_by_key(|i| i.id);
    b.sort_by_key(|i| i.id);

    for (x, y) in a.iter().zip(b.iter()) {
        let at = |field: &str| Some(format!("{field} of item {}", x.id));
        if x.id != y.id {
            return Some(format!("item id {} -> {}", x.id, y.id));
        }
        if x.title != y.title {
            return at("title");
        }
        if x.kind != y.kind {
            return Some(format!("kind of item {}: {:?} -> {:?}", x.id, x.kind, y.kind));
        }
        if x.body != y.body {
            return Some(format!(
                "body of item {}: {} chars -> {} chars",
                x.id,
                x.body.chars().count(),
                y.body.chars().count()
            ));
        }
        if x.username.as_deref().unwrap_or("") != y.username.as_deref().unwrap_or("") {
            return at("username");
        }
        if x.url.as_deref().unwrap_or("") != y.url.as_deref().unwrap_or("") {
            return at("url");
        }
        if x.workspace != y.workspace {
            return at("workspace");
        }
        if x.tags != y.tags {
            return Some(format!("tags of item {}: {:?} -> {:?}", x.id, x.tags, y.tags));
        }
        if x.use_count != y.use_count {
            return Some(format!(
                "use_count of item {}: {} -> {}",
                x.id, x.use_count, y.use_count
            ));
        }
        if !same_second(x.last_used_at, y.last_used_at) {
            return at("last_used_at");
        }
    }
    None
}

fn same_second(a: Option<DateTime<Utc>>, b: Option<DateTime<Utc>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => x.timestamp() == y.timestamp(),
        _ => false,
    }
}

// ---------- 備份與衝突副本 ----------

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string());
    let ext = path
        .extension()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "kdbx".to_string());
    let name = match stem {
        Some(s) => format!("{s}.{suffix}.{ext}"),
        None => format!("{suffix}.{ext}"),
    };
    path.with_file_name(name)
}

/// bak2 → bak3、bak1 → bak2、正本 → bak1。
fn rotate_backups(path: &Path) -> Result<(), VaultError> {
    if !path.exists() {
        return Ok(());
    }
    for n in (1..BACKUP_COUNT).rev() {
        let from = with_suffix(path, &format!("bak{n}"));
        let to = with_suffix(path, &format!("bak{}", n + 1));
        if from.exists() {
            std::fs::rename(&from, &to).map_err(|e| VaultError::io(&from, e))?;
        }
    }
    let first = with_suffix(path, "bak1");
    std::fs::copy(path, &first).map_err(|e| VaultError::io(path, e))?;
    Ok(())
}

/// 掃描同目錄的雲端衝突副本。
pub fn conflict_files(path: &Path) -> Vec<PathBuf> {
    let Some(dir) = path.parent() else {
        return Vec::new();
    };
    let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().to_lowercase()) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            if p == path {
                return false;
            }
            if p.extension().map(|e| e.to_string_lossy().to_lowercase()) != Some("kdbx".into()) {
                return false;
            }
            let Some(name) = p.file_stem().map(|s| s.to_string_lossy().to_lowercase()) else {
                return false;
            };
            if name == stem || !name.starts_with(&stem) {
                return false;
            }
            let rest = &name[stem.len()..];
            if rest.starts_with(".bak") || rest == ".tmp" {
                return false;
            }
            rest.contains("conflict")
                || rest.contains("衝突")
                || rest.starts_with('-')      // OneDrive：檔名-主機名稱
                || rest.trim_start().chars().all(|c| c.is_ascii_digit() || c == ' ')
            // iCloud：檔名 2
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 這三個鍵名已經寫進使用者的保險庫，改了就找不到第二層金鑰的材料。
    /// 見 `crypto::tests::frozen_format_constants`。
    #[test]
    fn frozen_custom_data_keys() {
        assert_eq!(CD_SECRET_SALT, "snipkit.secret.salt");
        assert_eq!(CD_SECRET_VERIFIER, "snipkit.secret.verifier");
        assert_eq!(CD_PUBLIC_KEY, "snipkit.secret.pubkey");
    }

    /// 寫出舊格式：kdbx 的密碼就是主密碼本身，密碼欄位是明文，沒有 CustomData。
    fn write_legacy_vault(path: &Path, password: &str, items: Vec<Item>) {
        let mut db = Database::new();
        db.config.kdf_config = KdfConfig::Argon2id {
            iterations: ARGON2_ITERATIONS,
            memory: ARGON2_MEMORY_BYTES,
            parallelism: ARGON2_PARALLELISM,
            version: argon2::Version::Version13,
        };
        let vault = Vault {
            path: path.to_path_buf(),
            vault_key: Zeroizing::new(password.to_string()),
            key: DatabaseKey::new().with_password(password),
            db,
            items,
        };
        vault.write_to(path).expect("寫舊格式");
    }

    fn temp_path(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "inkkeep-legacy-{tag}-{}",
            Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).expect("建暫存目錄");
        dir.join("vault.kdbx")
    }

    /// 舊保險庫用主密碼開得起來，而且開完就升級成兩層。
    #[test]
    fn a_legacy_vault_is_upgraded_in_place() {
        const PW: &str = "correct horse battery staple";
        let path = temp_path("upgrade");

        let mut pw_item = Item::new(ItemKind::Password, "GitHub", "hunter2");
        pw_item.username = Some("john".into());
        let pw_id = pw_item.id;
        let snip = Item::new(ItemKind::Snippet, "問候", "你好");
        write_legacy_vault(&path, PW, vec![pw_item, snip]);

        let vault = Vault::open_with_password(&path, PW).expect("舊保險庫要開得起來");

        assert_eq!(vault.vault_key(), crypto::derive_vault_key(PW).unwrap());
        assert!(
            matches!(Vault::open(&path, PW), Err(VaultError::WrongPassword)),
            "升級後主密碼不該再直接開得了檔案"
        );

        let body = &vault.get(pw_id).expect("找得到").body;
        assert!(crypto::is_encrypted(body), "升級後密碼欄位仍是明文");
        let secret = vault.unlock_secrets(PW).expect("第二層金鑰要備好了");
        assert_eq!(crypto::unseal(&secret, body).unwrap(), "hunter2");

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// 升級完之後還要能繼續正常寫入。
    #[test]
    fn a_vault_keeps_working_after_being_upgraded() {
        const PW: &str = "correct horse battery staple";
        let path = temp_path("upgrade-then-write");

        write_legacy_vault(&path, PW, vec![]);
        let mut vault = Vault::open_with_password(&path, PW).expect("升級");
        let secret = vault.unlock_secrets(PW).expect("第二層");

        vault
            .upsert(Item::new(ItemKind::Snippet, "問候", "你好 ${date}"))
            .expect("升級後存片語");

        let mut pw_item = Item::new(
            ItemKind::Password,
            "1234",
            crypto::encrypt(&secret, "Xk9#mQ2$vL7@pR4!aB3%").expect("加密"),
        );
        pw_item.username = Some("1234".into());
        vault.upsert(pw_item).expect("升級後存密碼");

        assert_eq!(vault.items().len(), 2);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    fn entry_field(vault: &Vault, id: Uuid, field: &str) -> String {
        vault
            .db
            .entry(keepass::db::EntryId::from_uuid(id))
            .and_then(|e| e.get(field).map(str::to_string))
            .unwrap_or_default()
    }

    fn set_entry_field(vault: &mut Vault, id: Uuid, field: &str, value: &str) {
        let mut entry = vault
            .db
            .entry_mut(keepass::db::EntryId::from_uuid(id))
            .expect("找得到 entry");
        entry.set_unprotected(field, value);
    }

    /// 換主密碼途中有一筆解不開：整個換不成，記憶體與檔案都還是舊密碼。
    #[test]
    fn a_failed_password_change_leaves_the_vault_untouched() {
        const OLD: &str = "old password";
        let path = temp_path("change-fails");
        let mut vault = Vault::create(&path, OLD).expect("建立");
        let public = vault.public_key().expect("公鑰");

        let good = Item::new(
            ItemKind::Password,
            "A",
            crypto::seal(&public, "hunter2").expect("封裝"),
        );
        let good_id = good.id;
        let good_body = good.body.clone();
        // 前綴對、內容壞掉，unseal 一定失敗
        let broken = Item::new(ItemKind::Password, "B", "snipkit-sealed:v1:!!!");
        vault.upsert_many(vec![good, broken]).expect("存");

        let old_secret = vault.unlock_secrets(OLD).expect("舊密碼");
        let old_key = vault.vault_key().to_string();
        assert!(vault.change_password(&old_secret, "new password").is_err());

        assert_eq!(vault.vault_key(), old_key, "記憶體裡的第一層金鑰被換掉了");
        assert_eq!(vault.get(good_id).unwrap().body, good_body, "記憶體裡的密碼被重包了");
        assert!(vault.unlock_secrets(OLD).is_ok(), "第二層材料被換掉了");
        let reopened = Vault::open(&path, &old_key).expect("檔案要還能用舊金鑰開");
        assert_eq!(reopened.get(good_id).unwrap().body, good_body);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// 換主密碼時，外部工具寫進來的明文密碼一併加密。
    #[test]
    fn a_password_change_seals_plaintext_passwords() {
        const OLD: &str = "old password";
        const NEW: &str = "new password";
        let path = temp_path("change-plaintext");
        let mut vault = Vault::create(&path, OLD).expect("建立");
        let plain = Item::new(ItemKind::Password, "A", "hunter2");
        let id = plain.id;
        vault.upsert(plain).expect("存");

        let old_secret = vault.unlock_secrets(OLD).expect("舊密碼");
        vault.change_password(&old_secret, NEW).expect("換密碼");

        let body = &vault.get(id).unwrap().body;
        assert!(crypto::is_encrypted(body));
        let new_secret = vault.unlock_secrets(NEW).expect("新密碼");
        assert_eq!(crypto::unseal(&new_secret, body).unwrap(), "hunter2");

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// 在其他 kdbx 工具加的備註、網址，InkKeep 存檔時不會清掉。
    #[test]
    fn saving_keeps_native_fields_the_item_does_not_use() {
        const PW: &str = "pw";
        let path = temp_path("keep-fields");
        let mut vault = Vault::create(&path, PW).expect("建立");
        let public = vault.public_key().expect("公鑰");

        let login = Item::new(
            ItemKind::Password,
            "GitHub",
            crypto::seal(&public, "hunter2").expect("封裝"),
        );
        let login_id = login.id;
        let snippet = Item::new(ItemKind::Snippet, "問候", "你好");
        let snippet_id = snippet.id;
        vault.upsert_many(vec![login, snippet]).expect("存");
        let key = vault.vault_key().to_string();

        // 模擬在 KeePassXC 補上備註與網址
        let mut vault = Vault::open(&path, &key).expect("開");
        set_entry_field(&mut vault, login_id, fields::NOTES, "恢復碼 1234");
        set_entry_field(&mut vault, snippet_id, fields::URL, "https://example.com");
        vault.save().expect("存");

        // 再開一次，做一次無關的寫入
        let mut vault = Vault::open(&path, &key).expect("開");
        assert_eq!(vault.get(snippet_id).unwrap().kind, ItemKind::Snippet);
        vault.touch(login_id).expect("記使用次數");

        let vault = Vault::open(&path, &key).expect("開");
        assert_eq!(entry_field(&vault, login_id, fields::NOTES), "恢復碼 1234");
        assert_eq!(entry_field(&vault, snippet_id, fields::URL), "https://example.com");
        assert_eq!(vault.get(login_id).unwrap().kind, ItemKind::Password);
        assert_eq!(vault.get(snippet_id).unwrap().kind, ItemKind::Snippet);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// 類型變了，舊類型用的欄位要清空，不殘留在看不到的地方。
    #[test]
    fn changing_kind_clears_the_old_fields() {
        const PW: &str = "pw";
        let path = temp_path("change-kind");
        let mut vault = Vault::create(&path, PW).expect("建立");
        let public = vault.public_key().expect("公鑰");

        let mut item = Item::new(
            ItemKind::Password,
            "A",
            crypto::seal(&public, "hunter2").expect("封裝"),
        );
        let id = item.id;
        vault.upsert(item.clone()).expect("存");
        let key = vault.vault_key().to_string();

        let mut vault = Vault::open(&path, &key).expect("開");
        set_entry_field(&mut vault, id, fields::NOTES, "備註");
        vault.save().expect("存");

        let mut vault = Vault::open(&path, &key).expect("開");
        item.kind = ItemKind::Snippet;
        item.body = "片語內容".into();
        vault.upsert(item).expect("改成片語");

        let vault = Vault::open(&path, &key).expect("開");
        assert_eq!(entry_field(&vault, id, fields::PASSWORD), "");
        assert_eq!(entry_field(&vault, id, fields::NOTES), "片語內容");

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// 舊保險庫加上錯的密碼，要回報密碼錯誤，不是「沒有第二層材料」之類的內部狀態。
    #[test]
    fn a_legacy_vault_with_the_wrong_password_reports_wrong_password() {
        let path = temp_path("upgrade-wrong");
        write_legacy_vault(&path, "right", vec![]);
        assert!(matches!(
            Vault::open_with_password(&path, "wrong"),
            Err(VaultError::WrongPassword)
        ));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
