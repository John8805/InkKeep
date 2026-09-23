//! kdbx 保險庫的整合測試。會在暫存目錄建真的檔案。
//!
//! Argon2id 64 MiB / t=3 每次開關檔都要跑一次 KDF，而兩層金鑰讓一次
//! `open_with_password` 要跑兩輪，所以這些測試比單元測試慢得多。

use inkkeep_core::crypto::{self, PUBLIC_KEY_LEN};
use inkkeep_core::model::{Item, ItemKind};
use inkkeep_core::vault::{conflict_files, Vault, VaultError, BACKUP_COUNT};
use std::path::{Path, PathBuf};

const PASSWORD: &str = "correct horse battery staple";

/// 用完自動刪掉的暫存目錄。
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "inkkeep-test-{tag}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&p).expect("建立暫存目錄");
        TempDir(p)
    }

    fn vault_path(&self) -> PathBuf {
        self.0.join("vault.kdbx")
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn snippet(title: &str, body: &str) -> Item {
    Item::new(ItemKind::Snippet, title, body)
}

fn bookmark(title: &str, url: &str) -> Item {
    Item::new(ItemKind::Bookmark, title, url)
}

/// 密碼項目的 body 一律是公鑰密文——封裝是上層的責任，保險庫只負責原樣搬運。
fn password(public: &[u8; PUBLIC_KEY_LEN], title: &str, user: &str, pass: &str) -> Item {
    let mut i = Item::new(ItemKind::Password, title, crypto::seal(public, pass).expect("封裝"));
    i.username = Some(user.into());
    i.url = Some(format!("{}.example.com", title.to_lowercase()));
    i
}

#[test]
fn create_then_open_roundtrips_all_three_kinds() {
    let dir = TempDir::new("kinds");
    let path = dir.vault_path();

    let mut vault = Vault::create(&path, PASSWORD).expect("建立");
    let secret = vault.unlock_secrets(PASSWORD).expect("解第二層");
    let public = crypto::public_key(&secret);
    vault
        .upsert(snippet("問候", "你好 ${date}"))
        .expect("存片語");
    vault
        .upsert(bookmark("文件", "https://example.com/docs"))
        .expect("存書籤");
    vault
        .upsert(password(&public, "GitHub", "john", "hunter2"))
        .expect("存密碼");

    let reopened = Vault::open_with_password(&path, PASSWORD).expect("重開");
    let items: Vec<&Item> = reopened.items().iter().collect();
    assert_eq!(items.len(), 3);

    let gh = items.iter().find(|i| i.title == "GitHub").unwrap();
    assert_eq!(gh.kind, ItemKind::Password);
    assert_eq!(gh.username.as_deref(), Some("john"));
    // 密碼帶了 URL 也不能被 detect_kind 誤判成書籤
    assert_eq!(gh.url.as_deref(), Some("github.example.com"));
    let secret = reopened.unlock_secrets(PASSWORD).expect("解第二層");
    assert_eq!(crypto::unseal(&secret, &gh.body).expect("拆封"), "hunter2");

    let doc = items.iter().find(|i| i.title == "文件").unwrap();
    assert_eq!(doc.kind, ItemKind::Bookmark);
    assert_eq!(doc.body, "https://example.com/docs");

    let greet = items.iter().find(|i| i.title == "問候").unwrap();
    assert_eq!(greet.kind, ItemKind::Snippet);
    assert_eq!(greet.body, "你好 ${date}");
}

/// 這是整個兩層設計的重點：拿第一層金鑰開檔，不必知道主密碼，
/// 片語與書籤就是明文，而密碼仍然是密文。
#[test]
fn the_vault_key_alone_opens_everything_except_passwords() {
    let dir = TempDir::new("tier1");
    let path = dir.vault_path();

    let mut vault = Vault::create(&path, PASSWORD).expect("建立");
    let secret = vault.unlock_secrets(PASSWORD).expect("解第二層");
    let public = crypto::public_key(&secret);
    vault.upsert(snippet("片語", "明文內容")).expect("存片語");
    vault
        .upsert(password(&public, "GitHub", "john", "hunter2"))
        .expect("存密碼");
    let vault_key = vault.vault_key().to_string();

    let reopened = Vault::open(&path, &vault_key).expect("用第一層金鑰重開");

    let snip = reopened.items().iter().find(|i| i.title == "片語").unwrap();
    assert_eq!(snip.body, "明文內容");

    let gh = reopened
        .items()
        .iter()
        .find(|i| i.title == "GitHub")
        .unwrap();
    assert!(
        crypto::is_encrypted(&gh.body),
        "只有第一層金鑰時密碼必須還是密文，實際拿到 {:?}",
        gh.body
    );
    assert!(!gh.body.contains("hunter2"));
    // 帳號不加密
    assert_eq!(gh.username.as_deref(), Some("john"));
}

/// 這條約束是 `model::validate` 那條「密碼不能空白」規則存在的理由。
///
/// kdbx 沒有存類型，是靠哪個欄位非空回推的。body 空白的密碼項目寫得進去，
/// 讀回來卻是片語——存檔後的回讀驗證會擋下來，但使用者看到的只會是
/// 一句莫名其妙的「驗證不一致」。所以要在更前面就擋。
#[test]
fn an_empty_password_comes_back_as_a_snippet() {
    let dir = TempDir::new("emptypw");
    let path = dir.vault_path();

    let mut vault = Vault::create(&path, PASSWORD).expect("建立");
    let mut item = Item::new(ItemKind::Password, "空的", "");
    item.username = Some("john".into());
    let id = item.id;

    // 繞過 validate 直接塞進去，重現「規則不存在的話會怎樣」
    let err = vault.upsert(item).expect_err("回讀驗證應該要擋下來");
    assert!(
        matches!(err, VaultError::VerifyFailed { .. }),
        "拿到 {err:?}"
    );

    // 候選檔留著，正本沒有被覆蓋
    let candidate = dir.path().join("vault.tmp.kdbx");
    assert!(candidate.exists(), "候選檔應該保留下來供檢查");
    let written = Vault::open(&candidate, vault.vault_key()).expect("候選檔本身是完整的");
    assert_eq!(
        written.get(id).expect("寫進去了").kind,
        ItemKind::Snippet,
        "空白密碼讀回來會被判成片語"
    );
}

/// 這是公鑰設計要換到的東西：**手上只有第一層金鑰也能新增密碼**。
///
/// 模擬使用者開了 app（第一層自動解鎖）但從沒輸入過主密碼的情況——
/// 公鑰從保險庫讀得到，封得起來，但讀不回內容。
#[test]
fn adding_a_password_needs_only_the_vault_key() {
    let dir = TempDir::new("writeonly");
    let path = dir.vault_path();
    let vault_key = Vault::create(&path, PASSWORD).expect("建立").vault_key().to_string();

    // 這一段完全沒有主密碼
    let mut vault = Vault::open(&path, &vault_key).expect("用第一層金鑰開");
    let public = vault.public_key().expect("公鑰要存在保險庫裡");
    let item = password(&public, "GitHub", "john", "hunter2");
    let id = item.id;
    vault.upsert(item).expect("只有公鑰也要存得進去");

    // 讀回來就得要主密碼了
    let reopened = Vault::open(&path, &vault_key).expect("重開");
    let body = &reopened.get(id).expect("找得到").body;
    assert!(crypto::is_encrypted(body));
    assert!(!body.contains("hunter2"));

    let secret = reopened.unlock_secrets(PASSWORD).expect("解第二層");
    assert_eq!(crypto::unseal(&secret, body).expect("拆封"), "hunter2");
}

/// 沒有公鑰欄位的保險庫，解鎖第二層時要補上。
#[test]
fn a_vault_without_a_public_key_gets_one_on_unlock() {
    let dir = TempDir::new("addpub");
    let path = dir.vault_path();
    let mut vault = Vault::create(&path, PASSWORD).expect("建立");
    let secret = vault.unlock_secrets(PASSWORD).expect("解第二層");

    assert!(!vault.ensure_public_key(&secret).expect("已經有了就不動"));
    assert_eq!(vault.public_key(), Some(crypto::public_key(&secret)));
}

#[test]
fn the_vault_key_is_the_same_on_every_machine() {
    let a = TempDir::new("stable-a");
    let b = TempDir::new("stable-b");
    let va = Vault::create(&a.vault_path(), PASSWORD).expect("建立 a");
    let vb = Vault::create(&b.vault_path(), PASSWORD).expect("建立 b");
    assert_eq!(
        va.vault_key(),
        vb.vault_key(),
        "同一組主密碼要推出同一把第一層金鑰，否則換裝置就開不了"
    );
}

#[test]
fn the_second_tier_rejects_the_wrong_password() {
    let dir = TempDir::new("tier2pw");
    let path = dir.vault_path();
    let vault = Vault::create(&path, PASSWORD).expect("建立");

    assert!(vault.unlock_secrets(PASSWORD).is_ok());
    assert!(
        matches!(
            vault.unlock_secrets("不是這個密碼"),
            Err(VaultError::WrongPassword)
        ),
        "保險庫裡沒有任何密碼項目時也要驗得出來"
    );
}

#[test]
fn wrong_password_is_reported_as_such() {
    let dir = TempDir::new("wrongpw");
    let path = dir.vault_path();
    Vault::create(&path, PASSWORD).expect("建立");

    match Vault::open_with_password(&path, "不是這個密碼") {
        Err(VaultError::WrongPassword) => {}
        other => panic!("預期 WrongPassword，拿到 {other:?}", other = other.err()),
    }
}

#[test]
fn opening_a_missing_file_reports_not_found() {
    let dir = TempDir::new("missing");
    let path = dir.vault_path();
    assert!(matches!(
        Vault::open_with_password(&path, PASSWORD),
        Err(VaultError::NotFound(_))
    ));
}

#[test]
fn changing_the_password_rekeys_both_tiers() {
    let dir = TempDir::new("rekey");
    let path = dir.vault_path();

    let mut vault = Vault::create(&path, PASSWORD).expect("建立");
    let secret = vault.unlock_secrets(PASSWORD).expect("解第二層");
    let public = crypto::public_key(&secret);
    let item = password(&public, "GitHub", "john", "hunter2");
    let id = item.id;
    vault.upsert(item).expect("存密碼");
    let old_vault_key = vault.vault_key().to_string();

    vault
        .change_password(&secret, "新的主密碼")
        .expect("換密碼");

    assert_ne!(vault.vault_key(), old_vault_key, "第一層金鑰要跟著換");
    assert!(
        matches!(
            Vault::open(&path, &old_vault_key),
            Err(VaultError::WrongPassword)
        ),
        "舊的第一層金鑰換完之後要開不了"
    );

    // 密碼欄位要用新的第二層金鑰重包過，否則換完就再也讀不回來
    let reopened = Vault::open_with_password(&path, "新的主密碼").expect("用新密碼開");
    let new_secret = reopened.unlock_secrets("新的主密碼").expect("解第二層");
    let body = &reopened.get(id).expect("找得到").body;
    assert_eq!(crypto::unseal(&new_secret, body).expect("拆封"), "hunter2");
}

#[test]
fn tags_and_usage_count_survive_a_roundtrip() {
    let dir = TempDir::new("meta");
    let path = dir.vault_path();

    let mut vault = Vault::create(&path, PASSWORD).expect("建立");
    let mut item = snippet("標籤測試", "內容");
    item.tags = vec!["工作".into(), "客服".into()];
    let id = item.id;
    vault.upsert(item).expect("存");

    vault.touch(id).expect("touch 1");
    vault.touch(id).expect("touch 2");

    let reopened = Vault::open(&path, vault.vault_key()).expect("重開");
    let item = reopened.get(id).expect("找得到");
    assert_eq!(item.tags, ["工作", "客服"]);
    assert_eq!(item.use_count, 2);
    assert!(item.last_used_at.is_some());
}

#[test]
fn changing_kind_clears_the_other_fields() {
    let dir = TempDir::new("kindswap");
    let path = dir.vault_path();

    let mut vault = Vault::create(&path, PASSWORD).expect("建立");
    let secret = vault.unlock_secrets(PASSWORD).expect("解第二層");
    let public = crypto::public_key(&secret);
    let mut item = password(&public, "暫時", "u", "secret");
    let id = item.id;
    vault.upsert(item.clone()).expect("存成密碼");

    // 改成片語：Password 欄位必須清掉，否則 detect_kind 會繼續判成 Password
    item.kind = ItemKind::Snippet;
    item.body = "現在是片語".into();
    item.username = None;
    vault.upsert(item).expect("改成片語");

    let reopened = Vault::open(&path, vault.vault_key()).expect("重開");
    let got = reopened.get(id).expect("找得到");
    assert_eq!(got.kind, ItemKind::Snippet);
    assert_eq!(got.body, "現在是片語");
}

#[test]
fn delete_removes_the_entry_from_the_file() {
    let dir = TempDir::new("delete");
    let path = dir.vault_path();

    let mut vault = Vault::create(&path, PASSWORD).expect("建立");
    let keep = snippet("留著", "a");
    let drop_it = snippet("刪掉", "b");
    let keep_id = keep.id;
    let drop_id = drop_it.id;
    vault.upsert(keep).expect("存 1");
    vault.upsert(drop_it).expect("存 2");

    vault.delete(drop_id).expect("刪");

    let reopened = Vault::open(&path, vault.vault_key()).expect("重開");
    assert_eq!(reopened.items().len(), 1);
    assert!(reopened.get(keep_id).is_some());
    assert!(reopened.get(drop_id).is_none());
}

#[test]
fn saving_rotates_backups_and_leaves_no_temp_file() {
    let dir = TempDir::new("backup");
    let path = dir.vault_path();

    let mut vault = Vault::create(&path, PASSWORD).expect("建立");
    for n in 0..4 {
        vault
            .upsert(snippet(&format!("第 {n} 筆"), "x"))
            .expect("存");
    }

    let bak = |n: usize| dir.path().join(format!("vault.bak{n}.kdbx"));
    assert!(bak(1).exists(), "缺 bak1");
    assert!(bak(2).exists(), "缺 bak2");
    assert!(bak(3).exists(), "缺 bak3");
    assert!(!bak(BACKUP_COUNT + 1).exists(), "備份份數超過上限");
    assert!(
        !dir.path().join("vault.tmp.kdbx").exists(),
        "暫存檔沒有被 rename 掉"
    );
}

#[test]
fn a_backup_is_a_readable_vault() {
    let dir = TempDir::new("bakread");
    let path = dir.vault_path();

    let mut vault = Vault::create(&path, PASSWORD).expect("建立");
    vault.upsert(snippet("第一筆", "內容")).expect("存 1");
    vault.upsert(snippet("第二筆", "內容")).expect("存 2");

    // bak1 是「存第二筆之前」的狀態，也就是只有第一筆
    let bak1 = dir.path().join("vault.bak1.kdbx");
    let restored = Vault::open(&bak1, vault.vault_key()).expect("備份可以開");
    assert_eq!(restored.items().len(), 1);
    assert_eq!(restored.items()[0].title, "第一筆");
}

#[test]
fn body_with_newlines_and_braces_survives() {
    let dir = TempDir::new("weird");
    let path = dir.vault_path();

    let body = "fn main() {\n    println!(\"{}\", 1);\n}\n\n--\nJohn";
    let mut vault = Vault::create(&path, PASSWORD).expect("建立");
    let item = snippet("程式碼", body);
    let id = item.id;
    vault.upsert(item).expect("存");

    let reopened = Vault::open(&path, vault.vault_key()).expect("重開");
    assert_eq!(reopened.get(id).unwrap().body, body);
}

#[test]
fn conflict_file_detection_ignores_our_own_backups() {
    let dir = TempDir::new("conflict");
    let path = dir.vault_path();
    let mut vault = Vault::create(&path, PASSWORD).expect("建立");
    vault.upsert(snippet("一筆", "x")).expect("存");

    // 造出各家雲端硬碟風格的衝突副本
    let make = |name: &str| {
        std::fs::copy(&path, dir.path().join(name)).expect("複製");
    };
    make("vault (conflicted copy 2026-09-22).kdbx"); // Dropbox
    make("vault-DESKTOP-ABC123.kdbx"); // OneDrive
    make("vault 2.kdbx"); // iCloud

    let found = conflict_files(&path);
    let mut names: Vec<String> = found
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    names.sort();

    assert_eq!(
        names,
        [
            "vault (conflicted copy 2026-09-22).kdbx",
            "vault 2.kdbx",
            "vault-DESKTOP-ABC123.kdbx",
        ]
    );
    assert!(
        !names.iter().any(|n| n.contains("bak")),
        "備份被誤判成衝突副本"
    );
}

#[test]
fn an_empty_vault_opens_cleanly() {
    let dir = TempDir::new("empty");
    let path = dir.vault_path();
    let vault = Vault::create(&path, PASSWORD).expect("建立");

    let reopened = Vault::open(&path, vault.vault_key()).expect("重開");
    assert!(reopened.items().is_empty());
}

// ---------- 工作區 ----------

/// 沒有工作區的保險庫，所有項目都在根群組。第一次開檔要建預設工作區、把全部收進去，而且要寫進檔案。
#[test]
fn ensure_workspaces_moves_everything_into_the_default() {
    let dir = TempDir::new("ws-ensure");
    let path = dir.vault_path();
    let mut vault = Vault::create(&path, PASSWORD).expect("建立");
    vault.upsert(snippet("問候", "你好")).expect("存");
    vault.upsert(bookmark("文件", "https://example.com")).expect("存");
    assert!(vault.items().iter().all(|i| i.workspace.is_none()));

    assert!(vault.ensure_workspaces("個人").expect("補工作區"), "第一次要有改動");
    let ws = vault.workspaces();
    assert_eq!(ws.len(), 1);
    assert_eq!(ws[0].name, "個人");

    let reopened = Vault::open(&path, vault.vault_key()).expect("重開");
    assert_eq!(reopened.workspaces(), ws, "工作區要寫進檔案");
    assert!(reopened.items().iter().all(|i| i.workspace == Some(ws[0].id)));

    let mut again = reopened;
    assert!(!again.ensure_workspaces("個人").expect("再補一次"), "第二次什麼都不該動");
    assert_eq!(again.workspaces().len(), 1, "不能重複建");
}

#[test]
fn moving_an_item_to_another_workspace_persists() {
    let dir = TempDir::new("ws-move");
    let path = dir.vault_path();
    let mut vault = Vault::create(&path, PASSWORD).expect("建立");
    vault.ensure_workspaces("個人").expect("補工作區");
    let work = vault.add_workspace("公司").expect("新增");

    let mut item = snippet("週報", "本週");
    item.workspace = Some(vault.workspaces()[0].id);
    let id = item.id;
    vault.upsert(item.clone()).expect("存");

    item.workspace = Some(work);
    vault.upsert(item).expect("搬");

    let reopened = Vault::open(&path, vault.vault_key()).expect("重開");
    assert_eq!(reopened.get(id).unwrap().workspace, Some(work));
}

/// kdbx 刪群組會連裡面的項目一起刪，所以非空的工作區一定要擋下來。
#[test]
fn a_workspace_with_items_cannot_be_deleted() {
    let dir = TempDir::new("ws-delete");
    let path = dir.vault_path();
    let mut vault = Vault::create(&path, PASSWORD).expect("建立");
    vault.ensure_workspaces("個人").expect("補工作區");
    let personal = vault.workspaces()[0].id;
    let work = vault.add_workspace("公司").expect("新增");

    let mut item = snippet("機密", "內容");
    item.workspace = Some(work);
    let id = item.id;
    vault.upsert(item).expect("存");

    assert!(matches!(
        vault.delete_workspace(work),
        Err(VaultError::WorkspaceNotEmpty(1))
    ));
    assert!(vault.get(id).is_some(), "項目不能被連帶刪掉");

    vault.delete_workspace(personal).expect("空的可以刪");
    assert!(matches!(
        vault.delete_workspace(work),
        Err(VaultError::WorkspaceNotEmpty(1))
    ));

    // 清空之後，最後一個也不能刪
    vault.delete(id).expect("刪項目");
    assert!(matches!(
        vault.delete_workspace(work),
        Err(VaultError::LastWorkspace)
    ));

    let reopened = Vault::open(&path, vault.vault_key()).expect("重開");
    assert_eq!(reopened.workspaces().len(), 1);
    assert_eq!(reopened.workspaces()[0].name, "公司");
}

#[test]
fn workspace_names_are_validated() {
    let dir = TempDir::new("ws-name");
    let path = dir.vault_path();
    let mut vault = Vault::create(&path, PASSWORD).expect("建立");
    vault.ensure_workspaces("個人").expect("補工作區");

    assert!(matches!(vault.add_workspace("  "), Err(VaultError::WorkspaceName("empty"))));
    assert!(matches!(vault.add_workspace("個人"), Err(VaultError::WorkspaceName("duplicate"))));
    let work = vault.add_workspace("Work").expect("新增");
    assert!(matches!(vault.add_workspace("work"), Err(VaultError::WorkspaceName("duplicate"))), "大小寫不分");
    assert!(matches!(
        vault.add_workspace(&"長".repeat(51)),
        Err(VaultError::WorkspaceName("too-long"))
    ));

    vault.rename_workspace(work, "公司").expect("改名");
    vault.rename_workspace(work, "公司").expect("改成自己原本的名字不算重複");
    let reopened = Vault::open(&path, vault.vault_key()).expect("重開");
    assert!(reopened.workspaces().iter().any(|w| w.id == work && w.name == "公司"));
}
