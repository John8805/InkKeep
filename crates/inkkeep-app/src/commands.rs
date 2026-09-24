//! 前端呼叫的 IPC command。

use crate::error::AppError;
use crate::insert;
use crate::settings::Settings;
use crate::state::AppState;
use chrono::Local;
use serde::{Deserialize, Serialize};
use inkkeep_core::gen::{self, GenOpts};
use inkkeep_core::import;
use inkkeep_core::password_import;
use inkkeep_core::model::{self, Item, ItemKind};
use inkkeep_core::search::{self, Index};
use inkkeep_core::template::{InsertPlan, ItemResolver, RenderCtx, Template, TemplateError};
use inkkeep_core::crypto;
use inkkeep_core::vault::{conflict_files, Vault};
use inkkeep_win::{clipboard, credstore};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

#[derive(Serialize)]
pub struct VaultState {
    /// 保險庫沒開。
    locked: bool,
    /// 第二層金鑰還沒推導，密碼項目仍是密文。
    secrets_locked: bool,
    exists: bool,
    path: String,
    item_count: usize,
    conflict_files: Vec<String>,
    /// 註冊失敗的快捷鍵；`None` 表示正常。
    hotkey_conflict: Option<String>,
}

#[derive(Serialize)]
pub struct SearchHit {
    id: String,
    title: String,
    kind: ItemKind,
    tags: Vec<String>,
    /// 依 kind 而異；Password 給 username 與網址，不給內容
    excerpt: String,
    sensitive: bool,
    /// 只對密碼項目有意義：有非空帳號可以送
    has_username: bool,
    /// 帳號原文，送帳號時送出去的就是這個
    username: Option<String>,
    workspace: Option<String>,
}

#[derive(Serialize)]
pub struct ItemView {
    id: String,
    title: String,
    kind: ItemKind,
    /// 密碼項目一律空字串；密碼原文由 `reveal_password` 取得。
    body: String,
    /// 只對密碼項目有意義：這一筆已經存了密碼。
    has_password: bool,
    username: Option<String>,
    /// 只有密碼項目有：這組帳密屬於哪個網站
    url: Option<String>,
    workspace: Option<String>,
    tags: Vec<String>,
    use_count: u32,
}

#[derive(Deserialize)]
pub struct ItemInput {
    id: Option<String>,
    title: String,
    kind: ItemKind,
    body: String,
    username: Option<String>,
    url: Option<String>,
    /// 沒給或給了不存在的就放第一個工作區
    workspace: Option<String>,
    tags: Vec<String>,
}

/// 依 title 找 Snippet 的 body。
struct Resolver(Vec<Item>);

impl ItemResolver for Resolver {
    fn resolve(&self, title: &str) -> Option<String> {
        let key = title.to_lowercase();
        self.0
            .iter()
            .filter(|i| i.kind.uses_template() && i.title.to_lowercase() == key)
            .max_by_key(|i| i.updated_at)
            .map(|i| i.body.clone())
    }
}

#[tauri::command]
pub fn vault_state(state: State<'_, AppState>) -> VaultState {
    let path = state.vault_path();
    VaultState {
        locked: state.is_locked(),
        secrets_locked: !state.secrets_unlocked(),
        exists: path.exists(),
        item_count: state.with_vault(|v, _| v.items().len()).unwrap_or(0),
        conflict_files: conflict_files(&path)
            .iter()
            .map(|p| p.display().to_string())
            .collect(),
        path: path.display().to_string(),
        hotkey_conflict: state.hotkey_conflict(),
    }
}

/// 用快取的第一層金鑰開保險庫。沒有快取就回 `None`。
///
/// 金鑰開不了檔（在別台裝置換過主密碼）時會把快取清掉。
pub fn open_with_cached_key(path: &Path) -> Option<Vault> {
    let key = credstore::load(path).ok().flatten()?;
    match Vault::open(path, &key) {
        Ok(vault) => Some(vault),
        Err(inkkeep_core::vault::VaultError::WrongPassword) => {
            let _ = credstore::forget(path);
            None
        }
        Err(_) => None,
    }
}

/// 確保保險庫至少有一個工作區，把還沒分配的項目收進第一個。開檔、建檔之後都要跑。
///
/// 存檔失敗不擋開檔：記憶體裡的分配已經做了，下一次任何存檔都會一起寫進去。
pub fn prepare_workspaces(app: &AppHandle, vault: &mut Vault) {
    let lang = crate::i18n::resolve(&Settings::load(app).language);
    if let Err(e) = vault.ensure_workspaces(crate::i18n::default_workspace(lang)) {
        eprintln!("補工作區時存檔失敗：{e}");
    }
}

/// 解析工作區 id。沒給或空字串是「全部」。
fn parse_workspace(raw: Option<&str>) -> Result<Option<Uuid>, AppError> {
    match raw.map(str::trim) {
        None | Some("") => Ok(None),
        Some(s) => Uuid::parse_str(s)
            .map(Some)
            .map_err(|_| AppError::WorkspaceNotFound),
    }
}

/// 決定目標工作區：指定的存在就用它，否則放第一個。
fn resolve_workspace(state: &AppState, wanted: Option<Uuid>) -> Result<Uuid, AppError> {
    let list = state
        .with_vault(|v, _| v.workspaces())
        .ok_or(AppError::Locked)?;
    if let Some(w) = wanted.filter(|w| list.iter().any(|x| x.id == *w)) {
        return Ok(w);
    }
    list.first().map(|w| w.id).ok_or(AppError::WorkspaceNotFound)
}

/// 用主密碼開保險庫，同一組主密碼一併解開第二層。
#[tauri::command]
pub fn unlock(
    app: AppHandle,
    password: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let path = state.vault_path();
    let mut vault = Vault::open_with_password(&path, &password)?;
    let secret = vault.unlock_secrets(&password)?;
    let _ = vault.ensure_public_key(&secret);
    prepare_workspaces(&app, &mut vault);

    if !Settings::load(&app).require_password_at_startup {
        // 存不進認證管理員時照常解鎖，下次啟動會再問主密碼
        let _ = credstore::store(&path, vault.vault_key());
    }
    state.unlock(vault);
    state.unlock_secrets(secret);
    Ok(())
}

/// 推導第二層金鑰。
#[tauri::command]
pub fn unlock_secrets(password: String, state: State<'_, AppState>) -> Result<(), AppError> {
    let secret = state
        .with_vault(|v, _| v.unlock_secrets(&password))
        .ok_or(AppError::Locked)??;

    // 保險庫缺公鑰時用這把金鑰補上；補不成照常解鎖，下次解鎖再補。
    let _ = state.with_vault_mut(|v| v.ensure_public_key(&secret));

    state.unlock_secrets(secret);
    Ok(())
}

/// 第一層金鑰的原文，能單獨開啟整個保險庫檔。要先解開第二層才給。
#[tauri::command]
pub fn reveal_vault_key(state: State<'_, AppState>) -> Result<String, AppError> {
    if !state.secrets_unlocked() {
        return Err(AppError::SecretsLocked);
    }
    state.vault_key().ok_or(AppError::Locked)
}

/// 把快取的第一層金鑰從認證管理員刪掉。下次啟動會改問主密碼。
#[tauri::command]
pub fn forget_vault_key(state: State<'_, AppState>) -> Result<(), AppError> {
    credstore::forget(&state.vault_path()).map_err(AppError::from)
}

#[tauri::command]
pub fn create_vault(
    app: AppHandle,
    password: String,
    with_samples: Option<bool>,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let path = state.vault_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| AppError::VaultIo {
            path: dir.display().to_string(),
            message: e.to_string(),
        })?;
    }
    let mut vault = Vault::create(&path, &password)?;
    if with_samples.unwrap_or(true) {
        vault.upsert_many(inkkeep_core::samples::samples())?;
    }
    prepare_workspaces(&app, &mut vault);
    let secret = vault.unlock_secrets(&password)?;
    if !Settings::load(&app).require_password_at_startup {
        let _ = credstore::store(&path, vault.vault_key());
    }
    state.unlock(vault);
    state.unlock_secrets(secret);
    Ok(())
}

/// 確認一個路徑能不能用。不建保險庫檔，但會嘗試建立上層資料夾。
#[tauri::command]
pub fn check_vault_path(path: String) -> Result<PathCheck, AppError> {
    let p = PathBuf::from(&path);
    let exists = p.exists();
    let parent_ok = p
        .parent()
        .map(|d| d.exists() || std::fs::create_dir_all(d).is_ok())
        .unwrap_or(false);
    Ok(PathCheck {
        exists,
        parent_ok,
        path: p.display().to_string(),
    })
}

#[derive(Serialize)]
pub struct PathCheck {
    exists: bool,
    /// 上層資料夾存在或建得起來
    parent_ok: bool,
    path: String,
}

/// 鎖定密碼：只清第二層金鑰，保險庫照常開著。
#[tauri::command]
pub fn lock(state: State<'_, AppState>) {
    state.lock_secrets();
}

#[tauri::command]
pub fn search_items(
    query: String,
    tags: Option<Vec<String>>,
    limit: Option<usize>,
    kind: Option<ItemKind>,
    workspace: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<SearchHit>, AppError> {
    state.touch_activity();
    let limit = limit.unwrap_or(search::DEFAULT_LIMIT);
    let workspace = parse_workspace(workspace.as_deref())?;
    state
        .with_vault(|vault, index| {
            let tags = tags.unwrap_or_default();
            to_hits(vault.items(), index, &query, &tags, kind, workspace, limit)
        })
        .ok_or(AppError::Locked)
}

fn to_hits(
    items: &[Item],
    index: &Index,
    query: &str,
    tags: &[String],
    kind: Option<ItemKind>,
    workspace: Option<Uuid>,
    limit: usize,
) -> Vec<SearchHit> {
    search::search(index, items, query, tags, kind, workspace, limit)
        .into_iter()
        .map(|i| {
            let item = &items[i];
            SearchHit {
                id: item.id.to_string(),
                title: item.title.clone(),
                kind: item.kind,
                tags: item.tags.clone(),
                excerpt: excerpt_of(item),
                sensitive: item.kind.is_sensitive(),
                has_username: item.username.as_deref().is_some_and(|u| !u.is_empty()),
                username: item.username.clone(),
                workspace: item.workspace.map(|w| w.to_string()),
            }
        })
        .collect()
}

/// Password 給 username 與網址，不含密碼內容。
fn excerpt_of(item: &Item) -> String {
    match item.kind {
        ItemKind::Password => {
            let mut parts: Vec<&str> = Vec::new();
            if let Some(u) = item.username.as_deref() {
                parts.push(u);
            }
            if let Some(u) = item.url.as_deref() {
                parts.push(u);
            }
            parts.join("  ")
        }
        _ => item
            .body
            .chars()
            .map(|c| if c == '\n' { '⏎' } else { c })
            .take(120)
            .collect(),
    }
}

/// 取單筆項目。**不解密**，不需第二層金鑰。
#[tauri::command]
pub fn item_get(id: String, state: State<'_, AppState>) -> Result<ItemView, AppError> {
    let uuid = parse_id(&id)?;
    let item = state
        .with_vault(|vault, _| vault.get(uuid).cloned())
        .ok_or(AppError::Locked)?
        .ok_or(AppError::NotFound { id })?;

    let is_password = item.kind == ItemKind::Password;
    Ok(ItemView {
        id: item.id.to_string(),
        title: item.title.clone(),
        kind: item.kind,
        body: if is_password {
            String::new()
        } else {
            item.body.clone()
        },
        has_password: is_password && !item.body.is_empty(),
        username: item.username.clone(),
        url: item.url.clone(),
        workspace: item.workspace.map(|w| w.to_string()),
        tags: item.tags.clone(),
        use_count: item.use_count,
    })
}

/// 回傳某一筆的密碼原文，要第二層金鑰。
#[tauri::command]
pub fn reveal_password(id: String, state: State<'_, AppState>) -> Result<String, AppError> {
    let uuid = parse_id(&id)?;
    let item = state
        .with_vault(|vault, _| vault.get(uuid).cloned())
        .ok_or(AppError::Locked)?
        .ok_or(AppError::NotFound { id })?;
    unseal_body(&state, &item.body)
}

/// 拆開密碼欄位。空字串直接回空字串，不需第二層金鑰。
pub fn unseal_body(state: &AppState, body: &str) -> Result<String, AppError> {
    if body.is_empty() {
        return Ok(String::new());
    }
    let key = state.secret().ok_or(AppError::SecretsLocked)?;
    Ok(crypto::unseal(&key, body)?)
}

#[tauri::command]
pub fn item_upsert(input: ItemInput, state: State<'_, AppState>) -> Result<String, AppError> {
    state.touch_activity();

    let mut item = match &input.id {
        Some(id) => {
            let uuid = parse_id(id)?;
            let mut existing = state
                .with_vault(|v, _| v.get(uuid).cloned())
                .ok_or(AppError::Locked)?
                .ok_or_else(|| AppError::NotFound { id: id.clone() })?;
            existing.updated_at = chrono::Utc::now();
            existing
        }
        None => Item::new(input.kind, "", ""),
    };
    // 留著原本的密文，密碼欄位留白時沿用
    let previous_sealed = (item.kind == ItemKind::Password).then(|| item.body.clone());

    item.title = input.title.trim().to_string();
    item.kind = input.kind;
    item.body = input.body;
    item.username = input.username.filter(|u| !u.trim().is_empty());
    item.url = if input.kind == ItemKind::Password {
        input.url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty())
    } else {
        // 書籤的網址就是 body，別的類型沒有這個欄位
        None
    };
    item.tags = input
        .tags
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    item.workspace = Some(resolve_workspace(
        &state,
        parse_workspace(input.workspace.as_deref())?,
    )?);

    if item.kind.uses_template() {
        Template::parse(&item.body)?;
    }

    // 密碼用公鑰封起來，**不需要主密碼**。密碼欄位留白代表「不改密碼」，
    // 沿用原本的密文。
    if item.kind == ItemKind::Password {
        if item.body.is_empty() {
            item.body = previous_sealed.unwrap_or_default();
        } else {
            let public = state
                .with_vault(|v, _| v.public_key())
                .ok_or(AppError::Locked)?
                // 保險庫還沒有公鑰：要解開第二層才會補上
                .ok_or(AppError::SecretsLocked)?;
            item.body = crypto::seal(&public, &item.body)?;
        }
    }

    // 驗證放在最後：密碼項目要檢查的是「封完之後 body 非空」，
    // 也就是使用者沒打密碼而且原本也沒有
    model::validate(&item)?;

    let id = item.id.to_string();
    state
        .with_vault_mut(|v| v.upsert(item))
        .ok_or(AppError::Locked)??;
    Ok(id)
}

#[tauri::command]
pub fn item_delete(id: String, state: State<'_, AppState>) -> Result<(), AppError> {
    state.touch_activity();
    let uuid = parse_id(&id)?;
    state
        .with_vault_mut(|v| v.delete(uuid))
        .ok_or(AppError::Locked)??;
    Ok(())
}

#[derive(Serialize)]
pub struct TemplateCheck {
    /// 文法錯誤，擋存檔。
    errors: Vec<TemplateError>,
    /// 跟保險庫內容有關的問題（找不到引用的片語、循環引用等），不擋存檔：
    /// 被引用的片語可能之後才建立。
    warnings: Vec<TemplateError>,
}

#[tauri::command]
pub fn template_validate(body: String, state: State<'_, AppState>) -> TemplateCheck {
    match Template::parse(&body) {
        Err(e) => TemplateCheck {
            errors: vec![e],
            warnings: Vec::new(),
        },
        Ok(template) => TemplateCheck {
            errors: Vec::new(),
            warnings: template
                .plan(&Resolver(state.items()))
                .err()
                .into_iter()
                .collect(),
        },
    }
}

#[tauri::command]
pub fn prepare_insert(id: String, state: State<'_, AppState>) -> Result<InsertPlan, AppError> {
    let uuid = parse_id(&id)?;
    let items = state.items();
    let item = items
        .iter()
        .find(|i| i.id == uuid)
        .ok_or_else(|| AppError::NotFound { id: id.clone() })?;

    if !item.kind.uses_template() {
        return Ok(InsertPlan {
            needs_inputs: Vec::new(),
            has_cursor: false,
        });
    }
    let template = Template::parse(&item.body)?;
    Ok(template.plan(&Resolver(items.clone()))?)
}

#[tauri::command]
pub fn preview(
    id: String,
    inputs: Option<BTreeMap<String, String>>,
    state: State<'_, AppState>,
) -> Result<String, AppError> {
    let uuid = parse_id(&id)?;
    let items = state.items();
    let item = items
        .iter()
        .find(|i| i.id == uuid)
        .ok_or_else(|| AppError::NotFound { id: id.clone() })?;

    if item.kind.is_sensitive() {
        return Ok("••••••••".into());
    }
    if !item.kind.uses_template() {
        return Ok(item.body.clone());
    }

    let template = Template::parse(&item.body)?;
    let ctx = RenderCtx::new(Local::now())
        .with_inputs(inputs.unwrap_or_default())
        .preview();
    Ok(template.render(&ctx, &Resolver(items.clone()))?.text)
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SendField {
    Body,
    Username,
    /// 密碼項目的網址
    Url,
}

#[tauri::command]
pub fn insert(
    app: AppHandle,
    id: String,
    field: Option<SendField>,
    inputs: Option<BTreeMap<String, String>>,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    state.touch_activity();
    insert::run(
        &app,
        &state,
        parse_id(&id)?,
        field.unwrap_or(SendField::Body),
        inputs.unwrap_or_default(),
    )
}

/// 用預設瀏覽器開啟書籤，開啟前收起視窗。只開 http、https 網址。
#[tauri::command]
pub fn open_bookmark(
    app: AppHandle,
    id: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    state.touch_activity();
    let uuid = parse_id(&id)?;
    let url = state
        .with_vault(|v, _| {
            v.get(uuid)
                .filter(|i| i.kind == ItemKind::Bookmark)
                .map(|i| i.body.clone())
        })
        .ok_or(AppError::Locked)?
        .ok_or(AppError::NotFound { id })?;
    if !inkkeep_win::browser::is_web_url(&url) {
        return Err(AppError::UnsupportedUrl);
    }
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
    inkkeep_win::browser::open_url(&url)?;
    let _ = state.with_vault_mut(|v| v.touch(uuid));
    Ok(())
}

#[tauri::command]
pub fn copy_only(
    id: String,
    field: Option<SendField>,
    inputs: Option<BTreeMap<String, String>>,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    state.touch_activity();
    let text = insert::render_for_send(
        &state,
        parse_id(&id)?,
        field.unwrap_or(SendField::Body),
        inputs.unwrap_or_default(),
    )?
    .text;
    clipboard::set_text(&text)?;
    Ok(())
}

#[tauri::command]
pub fn generate_password(opts: Option<GenOpts>) -> Result<String, AppError> {
    Ok(gen::generate_os(&opts.unwrap_or_default())?)
}

#[tauri::command]
pub fn entropy_bits(opts: GenOpts) -> f64 {
    opts.entropy_bits()
}

#[tauri::command]
pub fn hide_window(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
}

/// 換保險庫路徑，**並寫進 settings.toml**。
#[tauri::command]
pub fn set_vault_path(
    app: AppHandle,
    path: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let path = PathBuf::from(path);
    state.close();
    state.set_vault_path(path.clone());

    let mut settings = Settings::load(&app);
    settings.vault_path = Some(path.display().to_string());
    settings.save(&app).map_err(|e| AppError::VaultIo {
        path: Settings::path(&app).display().to_string(),
        message: e.to_string(),
    })
}

fn parse_id(id: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(id).map_err(|_| AppError::NotFound { id: id.to_string() })
}

// ---------- 設定、主密碼、匯入 ----------

#[tauri::command]
pub fn settings_get(app: AppHandle) -> Settings {
    Settings::load(&app)
}

#[tauri::command]
pub fn settings_set(
    app: AppHandle,
    settings: Settings,
    state: State<'_, AppState>,
) -> Result<Settings, AppError> {
    let saved = settings.sanitized();
    let previous = Settings::load(&app);
    if saved.hotkey != previous.hotkey && crate::apply_hotkey(&app, &saved.hotkey).is_err() {
        // 新的註冊不起來就換回舊的，設定也不存
        let _ = crate::apply_hotkey(&app, &previous.hotkey);
        return Err(AppError::Hotkey {
            combo: saved.hotkey,
        });
    }
    saved.save(&app).map_err(|e| AppError::VaultIo {
        path: Settings::path(&app).display().to_string(),
        message: e.to_string(),
    })?;
    state.set_lock_after_minutes(saved.lock_after_minutes);
    let new_path = saved.resolved_vault_path(&app);
    if new_path != state.vault_path() {
        state.close();
        state.set_vault_path(new_path);
    }
    if saved.require_password_at_startup {
        let _ = credstore::forget(&state.vault_path());
    } else if let Some(key) = state.vault_key() {
        let _ = credstore::store(&state.vault_path(), &key);
    }
    Ok(saved)
}

/// 換主密碼：兩層金鑰一起換，認證管理員裡的快取跟著更新。
#[tauri::command]
pub fn change_password(
    app: AppHandle,
    old: String,
    new: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    if new.trim().is_empty() {
        return Err(AppError::Validation(vec![
            inkkeep_core::model::ValidationError {
                field: "password".into(),
                message: "password-empty".into(),
            },
        ]));
    }

    // 用舊密碼推一次第二層當作驗證，順便拿到重新加密密碼欄位所需的金鑰。
    let old_secret = state
        .with_vault(|v, _| v.unlock_secrets(&old))
        .ok_or(AppError::Locked)??;

    state
        .with_vault_mut(|v| v.change_password(&old_secret, &new))
        .ok_or(AppError::Locked)??;

    let path = state.vault_path();
    if Settings::load(&app).require_password_at_startup {
        let _ = credstore::forget(&path);
    } else if let Some(key) = state.vault_key() {
        let _ = credstore::store(&path, &key);
    }

    // 第二層也換了，記憶體裡那把舊的已經解不開任何東西
    let new_secret = state
        .with_vault(|v, _| v.unlock_secrets(&new))
        .ok_or(AppError::Locked)??;
    state.unlock_secrets(new_secret);
    Ok(())
}

#[derive(Serialize)]
pub struct ImportPreview {
    total: usize,
    would_add: usize,
    would_skip: usize,
}

/// 這台電腦上所有可以匯入書籤的瀏覽器設定檔。
#[tauri::command]
pub fn import_sources() -> Vec<import::BrowserProfile> {
    import::list_profiles()
}

#[tauri::command]
pub fn import_preview(
    source: import::Source,
    path: Option<String>,
    workspace: Option<String>,
    state: State<'_, AppState>,
) -> Result<ImportPreview, AppError> {
    let bookmarks = import::read(source, path.as_ref().map(Path::new))?;
    let total = bookmarks.len();
    let target = resolve_workspace(&state, parse_workspace(workspace.as_deref())?)?;
    let existing = items_in(&state, target);
    let (added, report) = import::merge(&existing, bookmarks);
    Ok(ImportPreview {
        total,
        would_add: added.len(),
        would_skip: report.skipped,
    })
}

#[tauri::command]
pub fn import_bookmarks(
    source: import::Source,
    path: Option<String>,
    workspace: Option<String>,
    state: State<'_, AppState>,
) -> Result<import::ImportReport, AppError> {
    state.touch_activity();
    let bookmarks = import::read(source, path.as_ref().map(Path::new))?;
    let target = resolve_workspace(&state, parse_workspace(workspace.as_deref())?)?;
    let existing = items_in(&state, target);
    let (mut to_add, report) = import::merge(&existing, bookmarks);
    for item in &mut to_add {
        item.workspace = Some(target);
    }

    if !to_add.is_empty() {
        // 整批一次存檔，只跑一次備份輪替與驗證
        state
            .with_vault_mut(|v| v.upsert_many(to_add))
            .ok_or(AppError::Locked)??;
    }
    Ok(report)
}

/// 某個工作區裡的項目。
fn items_in(state: &AppState, workspace: Uuid) -> Vec<Item> {
    state
        .items()
        .into_iter()
        .filter(|i| i.workspace == Some(workspace))
        .collect()
}

// ---------- 工作區 ----------

#[derive(Serialize)]
pub struct WorkspaceView {
    id: String,
    name: String,
    /// 裡面有幾筆
    count: usize,
}

#[tauri::command]
pub fn workspace_list(state: State<'_, AppState>) -> Result<Vec<WorkspaceView>, AppError> {
    state
        .with_vault(|v, _| {
            v.workspaces()
                .into_iter()
                .map(|w| WorkspaceView {
                    count: v.items().iter().filter(|i| i.workspace == Some(w.id)).count(),
                    id: w.id.to_string(),
                    name: w.name,
                })
                .collect()
        })
        .ok_or(AppError::Locked)
}

#[tauri::command]
pub fn workspace_add(name: String, state: State<'_, AppState>) -> Result<String, AppError> {
    let id = state
        .with_vault_mut(|v| v.add_workspace(&name))
        .ok_or(AppError::Locked)??;
    Ok(id.to_string())
}

#[tauri::command]
pub fn workspace_rename(
    id: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let id = parse_workspace(Some(&id))?.ok_or(AppError::WorkspaceNotFound)?;
    state
        .with_vault_mut(|v| v.rename_workspace(id, &name))
        .ok_or(AppError::Locked)??;
    Ok(())
}

/// 只刪空的工作區，檢查在保險庫層。
#[tauri::command]
pub fn workspace_delete(id: String, state: State<'_, AppState>) -> Result<(), AppError> {
    let id = parse_workspace(Some(&id))?.ok_or(AppError::WorkspaceNotFound)?;
    state
        .with_vault_mut(|v| v.delete_workspace(id))
        .ok_or(AppError::Locked)??;
    Ok(())
}

// ---------- 密碼匯入 ----------

fn password_import_error(e: inkkeep_core::import::ImportError) -> AppError {
    use inkkeep_core::import::ImportError;
    match e {
        ImportError::Parse(m) if m == "not-a-password-export" => AppError::NotPasswordExport,
        other => AppError::from(other),
    }
}

/// 看看這個 CSV 會匯入幾筆。不需要任何金鑰，也不回傳任何密碼。
#[tauri::command]
pub fn password_import_preview(
    path: String,
    workspace: Option<String>,
    state: State<'_, AppState>,
) -> Result<password_import::Report, AppError> {
    let rows = password_import::read(Path::new(&path)).map_err(password_import_error)?;
    let target = resolve_workspace(&state, parse_workspace(workspace.as_deref())?)?;
    let existing = items_in(&state, target);
    let (_, report) = password_import::plan(&existing, rows);
    Ok(report)
}

/// 匯入密碼。用保險庫的公鑰封裝，**不需要主密碼**。
#[tauri::command]
pub fn password_import(
    path: String,
    workspace: Option<String>,
    state: State<'_, AppState>,
) -> Result<password_import::Report, AppError> {
    state.touch_activity();
    let rows = password_import::read(Path::new(&path)).map_err(password_import_error)?;
    let target = resolve_workspace(&state, parse_workspace(workspace.as_deref())?)?;
    let existing = items_in(&state, target);
    let (keep, report) = password_import::plan(&existing, rows);

    if !keep.is_empty() {
        let public = state
            .with_vault(|v, _| v.public_key())
            .ok_or(AppError::Locked)?
            // 保險庫還沒有公鑰：要解開第二層才會補上
            .ok_or(AppError::SecretsLocked)?;
        let mut items = password_import::seal(keep, &public)?;
        for item in &mut items {
            item.workspace = Some(target);
        }
        state
            .with_vault_mut(|v| v.upsert_many(items))
            .ok_or(AppError::Locked)??;
    }
    Ok(report)
}

/// 刪掉匯入完的密碼 CSV。只肯刪 `.csv`（副檔名不分大小寫）。
#[tauri::command]
pub fn delete_import_file(path: String) -> Result<(), AppError> {
    let p = PathBuf::from(&path);
    let is_csv = p
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("csv"));
    if !is_csv {
        return Err(AppError::Other {
            message: "only-csv".into(),
        });
    }
    std::fs::remove_file(&p).map_err(|e| AppError::VaultIo {
        path,
        message: e.to_string(),
    })
}

#[tauri::command]
pub fn tag_list(state: State<'_, AppState>) -> Vec<TagCount> {
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for item in state.items() {
        for tag in item.tags {
            *counts.entry(tag).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .map(|(name, count)| TagCount { name, count })
        .collect()
}

#[derive(Serialize)]
pub struct TagCount {
    name: String,
    count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_import_file_refuses_anything_but_csv() {
        let dir = std::env::temp_dir().join(format!("inkkeep-del-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let kdbx = dir.join("vault.kdbx");
        let csv = dir.join("export.CSV");
        std::fs::write(&kdbx, b"x").unwrap();
        std::fs::write(&csv, b"x").unwrap();

        assert!(delete_import_file(kdbx.display().to_string()).is_err());
        assert!(kdbx.exists(), "非 CSV 檔不能被刪");

        delete_import_file(csv.display().to_string()).expect("CSV 要刪得掉，副檔名不分大小寫");
        assert!(!csv.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
