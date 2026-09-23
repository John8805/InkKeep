//! 從其他密碼管理器匯出的 CSV 匯入密碼。
//!
//! 不綁定特定來源：看標題列的欄位名稱判斷哪一欄是什麼。各家匯出格式的欄位名稱
//! 不同但認得出來——Chrome／Edge 是 `name,url,username,password,note`，
//! Firefox 沒有標題欄只有 `url`，Bitwarden 是 `login_uri`、`login_password`，
//! 1Password、KeePassXC、LastPass 又各有一套。對照表在 [`Column::from_header`]。
//!
//! 匯入分兩步：[`plan`] 決定哪些要加、哪些跳過，**不需要任何金鑰**；
//! [`seal`] 用保險庫的公鑰把密碼封起來變成 `Item`，同樣不需要主密碼。
//!
//! 整個過程明文密碼只存在記憶體裡，不寫日誌。[`Report`] 只含筆數。

use crate::crypto::{self, CryptoError, PUBLIC_KEY_LEN};
use crate::import::{sanitize_tag, ImportError};
use crate::model::{self, Item, ItemKind};
use std::collections::HashSet;
use std::io::Read;
use std::path::Path;
use zeroize::Zeroize;

/// 一列匯入資料。還沒封裝，`password` 是明文。
pub struct Row {
    pub title: String,
    pub url: String,
    pub username: String,
    pub password: String,
    pub notes: String,
    pub folders: Vec<String>,
}

impl Drop for Row {
    fn drop(&mut self) {
        self.password.zeroize();
    }
}

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct Report {
    /// 檔案裡的登入項目總數（已排除 Bitwarden 的卡片、筆記等非登入類型）
    pub total: usize,
    pub added: usize,
    /// 網域加帳號跟既有項目或同一批裡的前一筆重複
    pub duplicates: usize,
    /// 密碼欄是空的而跳過
    pub no_password: usize,
    /// 已匯入、但備註沒有跟著匯入的筆數
    pub notes_dropped: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Title,
    Url,
    Username,
    Password,
    Notes,
    Folder,
    Kind,
    Other,
}

impl Column {
    /// 欄位名稱正規化成小寫、只留英數字之後對照。
    /// `login_uri`、`Login URI`、`LoginURI` 都會變成 `loginuri`。
    fn from_header(raw: &str) -> Column {
        let key: String = raw
            .trim_start_matches('\u{feff}') // Excel、1Password 會在檔頭放 BOM
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        match key.as_str() {
            "name" | "title" => Column::Title,
            "url" | "loginuri" | "website" | "origin" => Column::Url,
            "username" | "loginusername" | "login" | "user" => Column::Username,
            "password" | "loginpassword" => Column::Password,
            "note" | "notes" | "extra" | "comments" => Column::Notes,
            "folder" | "grouping" | "group" => Column::Folder,
            "type" => Column::Kind,
            _ => Column::Other,
        }
    }
}

pub fn read(path: &Path) -> Result<Vec<Row>, ImportError> {
    if !path.exists() {
        return Err(ImportError::NotFound(path.to_path_buf()));
    }
    let file = std::fs::File::open(path).map_err(|e| ImportError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    parse(file)
}

/// 解析 CSV。看不出密碼欄在哪就當作不是密碼匯出檔，回傳錯誤而不是空清單。
pub fn parse(reader: impl Read) -> Result<Vec<Row>, ImportError> {
    let mut csv = csv::ReaderBuilder::new()
        .flexible(true) // 有些匯出檔某幾列欄位數不一致
        .from_reader(reader);

    let headers: Vec<Column> = csv
        .headers()
        .map_err(|e| ImportError::Parse(e.to_string()))?
        .iter()
        .map(Column::from_header)
        .collect();
    if !headers.contains(&Column::Password) {
        return Err(ImportError::Parse("not-a-password-export".into()));
    }

    let mut rows = Vec::new();
    for record in csv.records() {
        let record = record.map_err(|e| ImportError::Parse(e.to_string()))?;
        let mut row = Row {
            title: String::new(),
            url: String::new(),
            username: String::new(),
            password: String::new(),
            notes: String::new(),
            folders: Vec::new(),
        };
        let mut is_login = true;
        for (col, value) in headers.iter().zip(record.iter()) {
            match col {
                Column::Title => row.title = value.trim().to_string(),
                Column::Url => row.url = value.trim().to_string(),
                Column::Username => row.username = value.trim().to_string(),
                // 密碼不 trim：前後空白可能就是密碼的一部分
                Column::Password => row.password = value.to_string(),
                Column::Notes => row.notes = value.trim().to_string(),
                Column::Folder => row.folders = folder_tags(value),
                // Bitwarden 的匯出混著卡片、安全筆記、身分資料，只要登入類型
                Column::Kind => is_login = value.trim().is_empty() || value.trim() == "login",
                Column::Other => {}
            }
        }
        if is_login {
            rows.push(row);
        }
    }
    Ok(rows)
}

/// `工作/專案`、`Root\工作` 這類資料夾路徑拆成標籤。KeePassXC 的根群組叫 Root，不當標籤。
fn folder_tags(path: &str) -> Vec<String> {
    path.split(['/', '\\'])
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("root"))
        .map(sanitize_tag)
        .filter(|s| !s.is_empty())
        .collect()
}

/// 用來判斷重複的鍵：網域加帳號。網址的路徑、協定、`www.` 都不算。
fn dedupe_key(url: &str, username: &str, title: &str) -> String {
    let host = site_of(url).unwrap_or_else(|| title.trim().to_lowercase());
    format!("{}\n{}", host, username.trim().to_lowercase())
}

fn site_of(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    // Firefox 會匯出 `https://example.com`，也有工具只給 `example.com`
    let parsed = url::Url::parse(url)
        .or_else(|_| url::Url::parse(&format!("https://{url}")))
        .ok()?;
    let host = parsed.host_str()?.to_lowercase();
    Some(host.strip_prefix("www.").unwrap_or(&host).to_string())
}

/// 決定哪些列要匯入，不碰金鑰。
pub fn plan(existing: &[Item], rows: Vec<Row>) -> (Vec<Row>, Report) {
    let mut seen: HashSet<String> = existing
        .iter()
        .filter(|i| i.kind == ItemKind::Password)
        .map(|i| {
            dedupe_key(
                i.url.as_deref().unwrap_or(""),
                i.username.as_deref().unwrap_or(""),
                &i.title,
            )
        })
        .collect();

    let mut report = Report {
        total: rows.len(),
        ..Report::default()
    };
    let mut keep = Vec::new();
    for mut row in rows {
        if row.password.is_empty() {
            report.no_password += 1;
            continue;
        }
        // Firefox 沒有名稱欄：用網域當標題，再不行用帳號
        if row.title.is_empty() {
            row.title = site_of(&row.url).unwrap_or_else(|| row.username.clone());
        }
        if row.title.is_empty() {
            row.title = "(untitled)".into();
        }
        if !seen.insert(dedupe_key(&row.url, &row.username, &row.title)) {
            report.duplicates += 1;
            continue;
        }
        if !row.notes.is_empty() {
            report.notes_dropped += 1;
        }
        keep.push(row);
    }
    report.added = keep.len();
    (keep, report)
}

/// 用保險庫的公鑰把密碼封起來。**不需要主密碼。**
pub fn seal(rows: Vec<Row>, public: &[u8; PUBLIC_KEY_LEN]) -> Result<Vec<Item>, CryptoError> {
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let title: String = row.title.chars().take(model::MAX_TITLE_LEN).collect();
        let mut item = Item::new(ItemKind::Password, title, crypto::seal(public, &row.password)?);
        item.username = (!row.username.is_empty()).then(|| row.username.clone());
        item.url = (!row.url.is_empty()).then(|| row.url.clone());
        item.tags = row.folders.clone();
        items.push(item);
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(csv: &str) -> Vec<Row> {
        parse(csv.as_bytes()).expect("解析")
    }

    #[test]
    fn chrome_export() {
        let got = rows(
            "name,url,username,password,note\n\
             github.com,https://github.com/login,john,hunter2,\n",
        );
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].title, "github.com");
        assert_eq!(got[0].url, "https://github.com/login");
        assert_eq!(got[0].username, "john");
        assert_eq!(got[0].password, "hunter2");
    }

    /// Firefox 沒有名稱欄，標題要從網域補。
    #[test]
    fn firefox_export_has_no_title_column() {
        let parsed = rows(
            "\"url\",\"username\",\"password\",\"httpRealm\",\"formActionOrigin\",\"guid\"\n\
             \"https://www.example.com\",\"john\",\"s3cret\",,\"https://www.example.com\",\"{abc}\"\n",
        );
        let (keep, _) = plan(&[], parsed);
        assert_eq!(keep[0].title, "example.com");
    }

    /// Bitwarden 的欄位名稱不一樣，而且混著非登入的項目；資料夾要變成標籤。
    #[test]
    fn bitwarden_export() {
        let got = rows(
            "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n\
             工作,,login,GitLab,,,0,https://gitlab.com,john,pw1,\n\
             ,,note,我的筆記,內容,,0,,,,\n\
             個人/社群,,login,X,,,0,https://x.com,jj,pw2,\n",
        );
        assert_eq!(got.len(), 2, "安全筆記不該被匯入");
        assert_eq!(got[0].url, "https://gitlab.com");
        assert_eq!(got[0].password, "pw1");
        assert_eq!(got[0].folders, ["工作"]);
        assert_eq!(got[1].folders, ["個人", "社群"]);
    }

    /// KeePassXC：根群組 Root 不當標籤，帶空白的群組名要換掉空白才是合法標籤。
    #[test]
    fn keepassxc_export() {
        let got = rows(
            "\"Group\",\"Title\",\"Username\",\"Password\",\"URL\",\"Notes\"\n\
             \"Root/Work Stuff\",\"VPN\",\"john\",\"pw\",\"vpn.corp\",\"備註\"\n",
        );
        assert_eq!(got[0].folders, ["Work-Stuff"]);
        assert_eq!(got[0].notes, "備註");
    }

    /// 1Password 等工具會在檔頭放 BOM，第一欄的名稱前面會多一個看不見的字元。
    #[test]
    fn byte_order_mark_is_ignored() {
        let got = rows("\u{feff}Title,Url,Username,Password\nA,a.com,u,p\n");
        assert_eq!(got[0].title, "A");
    }

    /// 密碼裡的逗號、引號、換行都要原樣保留，前後空白也不能被吃掉。
    #[test]
    fn tricky_passwords_survive() {
        let got = rows("name,url,username,password\nx,x.com,u,\" a,b\"\"c\nd \"\n");
        assert_eq!(got[0].password, " a,b\"c\nd ");
    }

    /// 選錯檔案要報錯，不是「成功匯入 0 筆」。
    #[test]
    fn a_file_without_a_password_column_is_rejected() {
        let err = parse("title,url\nA,a.com\n".as_bytes()).err().expect("應該失敗");
        assert!(matches!(err, ImportError::Parse(ref m) if m == "not-a-password-export"));
    }

    #[test]
    fn duplicates_and_empty_passwords_are_skipped() {
        let mut existing = Item::new(ItemKind::Password, "GitHub", "sealed");
        existing.url = Some("https://github.com".into());
        existing.username = Some("john".into());

        let (keep, report) = plan(
            &[existing],
            rows(
                "name,url,username,password,note\n\
                 gh,https://www.github.com/login,JOHN,x,\n\
                 gl,https://gitlab.com,john,y,有備註\n\
                 gl2,gitlab.com,john,z,\n\
                 empty,https://e.com,john,,\n",
            ),
        );
        assert_eq!(report.total, 4);
        assert_eq!(report.duplicates, 2, "www、路徑、大小寫都不影響判斷");
        assert_eq!(report.no_password, 1);
        assert_eq!(report.notes_dropped, 1);
        assert_eq!(report.added, 1);
        assert_eq!(keep[0].title, "gl");
    }

    /// 封裝只需要公鑰，封完要拆得回來。
    #[test]
    fn sealed_items_open_with_the_secret_key() {
        let secret = crypto::derive_secret_key("master", &[1u8; crypto::SECRET_SALT_LEN]).unwrap();
        let public = crypto::public_key(&secret);
        let (keep, _) = plan(&[], rows("name,url,username,password\nA,a.com,u,hunter2\n"));
        let items = seal(keep, &public).unwrap();

        assert_eq!(items[0].kind, ItemKind::Password);
        assert!(crypto::is_encrypted(&items[0].body));
        assert!(!items[0].body.contains("hunter2"));
        assert_eq!(crypto::unseal(&secret, &items[0].body).unwrap(), "hunter2");
        assert!(model::validate(&items[0]).is_ok());
    }
}
