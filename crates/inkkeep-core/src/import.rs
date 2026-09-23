//! 瀏覽器書籤匯入。
//!
//! 一次性匯入：資料夾路徑轉成標籤，URL 重複的跳過。

use crate::model::{Item, ItemKind};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Chrome,
    Edge,
    Firefox,
    /// 直接指定檔案，格式依副檔名判斷
    File,
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("找不到書籤檔：{0}")]
    NotFound(PathBuf),
    #[error("讀取 {path} 失敗：{source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("書籤檔格式無法解析：{0}")]
    Parse(String),
}

#[derive(Debug, Default, serde::Serialize)]
pub struct ImportReport {
    pub added: usize,
    /// URL 與既有項目重複
    pub skipped: usize,
}

/// 一筆待匯入的書籤，尚未轉成 `Item`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub title: String,
    pub url: String,
    /// 由資料夾階層而來，已去掉根節點
    pub folders: Vec<String>,
}

impl Bookmark {
    fn into_item(self) -> Item {
        let mut item = Item::new(ItemKind::Bookmark, self.title, self.url);
        item.tags = self.folders;
        item
    }
}

/// 一個可以匯入書籤的瀏覽器設定檔。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BrowserProfile {
    pub source: Source,
    /// 顯示名稱，例如「Chrome — John」，後半取自瀏覽器設定檔自己的名稱
    pub label: String,
    pub path: PathBuf,
}

/// 列出這台電腦上所有有書籤的瀏覽器設定檔。
pub fn list_profiles() -> Vec<BrowserProfile> {
    let mut out = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        out.extend(chromium_profiles(
            Source::Chrome,
            "Chrome",
            &local.join("Google").join("Chrome").join("User Data"),
        ));
        out.extend(chromium_profiles(
            Source::Edge,
            "Edge",
            &local.join("Microsoft").join("Edge").join("User Data"),
        ));
    }
    if let Some(roaming) = std::env::var_os("APPDATA").map(PathBuf::from) {
        out.extend(firefox_profiles(&roaming.join("Mozilla").join("Firefox")));
    }
    out
}

/// Chromium 系（Chrome、Edge）：每個設定檔是 `User Data` 底下一個資料夾，
/// 顯示名稱在 `Local State` 的 `profile.info_cache.<資料夾>.name`。
fn chromium_profiles(source: Source, browser: &str, user_data: &Path) -> Vec<BrowserProfile> {
    let names: std::collections::HashMap<String, String> =
        std::fs::read_to_string(user_data.join("Local State"))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|v| v.get("profile")?.get("info_cache")?.as_object().cloned())
            .map(|cache| {
                cache
                    .into_iter()
                    .filter_map(|(dir, info)| Some((dir, info.get("name")?.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default();

    let Ok(entries) = std::fs::read_dir(user_data) else {
        return Vec::new();
    };
    // 讀得到 Local State 就只列它登記的設定檔——訪客設定檔也可能有書籤檔，
    // 但它不是使用者的設定檔，不在 info_cache 裡。讀不到才退回「有書籤檔就算」，
    // 並用名稱排除訪客與系統設定檔。
    let mut dirs: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().join("Bookmarks").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|d| {
            if names.is_empty() {
                d != "Guest Profile" && d != "System Profile"
            } else {
                names.contains_key(d)
            }
        })
        .collect();
    // Default 先，其餘照 Profile 後面的數字排——字典序會讓 Profile 10 排在 Profile 2 前面
    dirs.sort_by_key(|d| {
        let n = d
            .strip_prefix("Profile ")
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or(u32::MAX);
        (d != "Default", n, d.clone())
    });

    dirs.into_iter()
        .map(|dir| BrowserProfile {
            source,
            label: format!(
                "{browser} — {}",
                names
                    .get(&dir)
                    .filter(|n| !n.trim().is_empty())
                    .cloned()
                    .unwrap_or_else(|| dir.clone())
            ),
            path: user_data.join(&dir).join("Bookmarks"),
        })
        .collect()
}

/// Firefox：設定檔清單在 `profiles.ini`，每一段 `[ProfileN]` 有 `Name`、`Path`、`IsRelative`。
/// 讀不到 profiles.ini 就退回掃描 `Profiles` 資料夾。
fn firefox_profiles(firefox_dir: &Path) -> Vec<BrowserProfile> {
    let profile = |label: String, dir: PathBuf| {
        let path = dir.join("places.sqlite");
        path.is_file().then(|| BrowserProfile {
            source: Source::Firefox,
            label: format!("Firefox — {label}"),
            path,
        })
    };

    let Ok(ini) = std::fs::read_to_string(firefox_dir.join("profiles.ini")) else {
        let Ok(entries) = std::fs::read_dir(firefox_dir.join("Profiles")) else {
            return Vec::new();
        };
        return entries
            .filter_map(Result::ok)
            .filter_map(|e| profile(e.file_name().to_string_lossy().into_owned(), e.path()))
            .collect();
    };

    #[derive(Default)]
    struct Section {
        is_profile: bool,
        name: Option<String>,
        path: Option<String>,
        relative: bool,
    }
    let mut sections: Vec<Section> = Vec::new();
    for line in ini.lines().map(str::trim) {
        if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            sections.push(Section {
                is_profile: header.starts_with("Profile"),
                relative: true,
                ..Section::default()
            });
        } else if let (Some(current), Some((key, value))) = (sections.last_mut(), line.split_once('=')) {
            match key {
                "Name" => current.name = Some(value.to_string()),
                "Path" => current.path = Some(value.to_string()),
                "IsRelative" => current.relative = value != "0",
                _ => {}
            }
        }
    }

    sections
        .into_iter()
        .filter(|s| s.is_profile)
        .filter_map(|s| {
            let path = s.path?;
            let dir = if s.relative {
                firefox_dir.join(path.replace('/', "\\"))
            } else {
                PathBuf::from(path)
            };
            profile(s.name.unwrap_or_else(|| dir.display().to_string()), dir)
        })
        .collect()
}

/// 各瀏覽器的預設書籤檔位置。
pub fn default_path(source: Source) -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    match source {
        Source::Chrome => Some(
            local
                .join("Google")
                .join("Chrome")
                .join("User Data")
                .join("Default")
                .join("Bookmarks"),
        ),
        Source::Edge => Some(
            local
                .join("Microsoft")
                .join("Edge")
                .join("User Data")
                .join("Default")
                .join("Bookmarks"),
        ),
        // Firefox 的設定檔目錄名稱帶隨機字串，要掃描
        Source::Firefox => firefox_places(),
        Source::File => None,
    }
}

fn firefox_places() -> Option<PathBuf> {
    let roaming = std::env::var_os("APPDATA").map(PathBuf::from)?;
    let profiles = roaming.join("Mozilla").join("Firefox").join("Profiles");
    let entries = std::fs::read_dir(&profiles).ok()?;
    entries
        .filter_map(Result::ok)
        .map(|e| e.path().join("places.sqlite"))
        .find(|p| p.exists())
}

/// 讀出書籤清單，不碰保險庫。
pub fn read(source: Source, path: Option<&Path>) -> Result<Vec<Bookmark>, ImportError> {
    let path = match path {
        Some(p) => p.to_path_buf(),
        None => default_path(source).ok_or_else(|| ImportError::NotFound(PathBuf::new()))?,
    };
    if !path.exists() {
        return Err(ImportError::NotFound(path));
    }

    let is_sqlite = source == Source::Firefox
        || path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("sqlite"));

    if is_sqlite {
        read_firefox(&path)
    } else {
        let text = std::fs::read_to_string(&path).map_err(|e| ImportError::Io {
            path: path.clone(),
            source: e,
        })?;
        parse_chromium(&text)
    }
}

/// 把書籤合併進既有項目清單，回傳新增的項目與統計。
///
/// 比對用正規化後的 URL：只差尾端斜線或大小寫的視為同一筆。
pub fn merge(existing: &[Item], bookmarks: Vec<Bookmark>) -> (Vec<Item>, ImportReport) {
    use std::collections::HashSet;

    let mut seen: HashSet<String> = existing
        .iter()
        .filter(|i| i.kind == ItemKind::Bookmark)
        .map(|i| normalize_url(&i.body))
        .collect();

    let mut report = ImportReport::default();
    let mut out = Vec::new();
    for bm in bookmarks {
        let key = normalize_url(&bm.url);
        if !seen.insert(key) {
            report.skipped += 1;
            continue;
        }
        report.added += 1;
        out.push(bm.into_item());
    }
    (out, report)
}

fn normalize_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_lowercase()
}

// ---------- Chromium（Chrome / Edge）----------

#[derive(Deserialize)]
struct ChromiumFile {
    roots: std::collections::BTreeMap<String, ChromiumNode>,
}

#[derive(Deserialize)]
struct ChromiumNode {
    #[serde(default)]
    name: String,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    children: Vec<ChromiumNode>,
}

fn parse_chromium(text: &str) -> Result<Vec<Bookmark>, ImportError> {
    let file: ChromiumFile =
        serde_json::from_str(text).map_err(|e| ImportError::Parse(e.to_string()))?;
    let mut out = Vec::new();
    for root in file.roots.values() {
        // 根節點（書籤列、其他書籤）本身不當標籤
        for child in &root.children {
            walk_chromium(child, &[], &mut out);
        }
    }
    Ok(out)
}

fn walk_chromium(node: &ChromiumNode, folders: &[String], out: &mut Vec<Bookmark>) {
    match node.kind.as_str() {
        "url" => {
            if let Some(url) = &node.url {
                if url.starts_with("http") {
                    out.push(Bookmark {
                        title: fallback_title(&node.name, url),
                        url: url.clone(),
                        folders: folders.to_vec(),
                    });
                }
            }
        }
        "folder" => {
            let mut nested = folders.to_vec();
            if !node.name.trim().is_empty() {
                nested.push(sanitize_tag(&node.name));
            }
            for child in &node.children {
                walk_chromium(child, &nested, out);
            }
        }
        _ => {}
    }
}

// ---------- Firefox ----------

fn read_firefox(path: &Path) -> Result<Vec<Bookmark>, ImportError> {
    // Firefox 執行中會鎖住 places.sqlite，先複製到暫存再讀。
    // Firefox 用 WAL 模式，還沒寫回主檔的新書籤在 `-wal` 檔裡，要一起複製；
    // SQLite 開檔時依「主檔名-wal」找到它並套用。
    let dir = std::env::temp_dir().join(format!("inkkeep-places-{}", uuid::Uuid::new_v4()));
    let copy = |from: &Path, to: &Path| {
        std::fs::copy(from, to).map_err(|e| ImportError::Io {
            path: from.to_path_buf(),
            source: e,
        })
    };
    let result = (|| {
        std::fs::create_dir_all(&dir).map_err(|e| ImportError::Io {
            path: dir.clone(),
            source: e,
        })?;
        let tmp = dir.join("places.sqlite");
        copy(path, &tmp)?;
        let wal = sidecar(path, "-wal");
        if wal.exists() {
            copy(&wal, &sidecar(&tmp, "-wal"))?;
        }
        read_firefox_inner(&tmp)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// `places.sqlite` → `places.sqlite-wal` 這類 SQLite 附屬檔的路徑。
fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn read_firefox_inner(path: &Path) -> Result<Vec<Bookmark>, ImportError> {
    use rusqlite::Connection;
    use std::collections::HashMap;

    let conn = Connection::open(path).map_err(|e| ImportError::Parse(e.to_string()))?;

    // 先讀所有資料夾，建 id → (名稱, 父 id) 的表，之後才能還原階層
    let mut folders: HashMap<i64, (String, i64)> = HashMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT id, COALESCE(title, ''), parent FROM moz_bookmarks WHERE type = 2")
            .map_err(|e| ImportError::Parse(e.to_string()))?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .map_err(|e| ImportError::Parse(e.to_string()))?;
        for row in rows.flatten() {
            folders.insert(row.0, (row.1, row.2));
        }
    }

    let mut stmt = conn
        .prepare(
            "SELECT COALESCE(b.title, ''), p.url, b.parent
             FROM moz_bookmarks b JOIN moz_places p ON b.fk = p.id
             WHERE b.type = 1 AND p.url LIKE 'http%'",
        )
        .map_err(|e| ImportError::Parse(e.to_string()))?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .map_err(|e| ImportError::Parse(e.to_string()))?;

    let mut out = Vec::new();
    for (title, url, parent) in rows.flatten() {
        out.push(Bookmark {
            title: fallback_title(&title, &url),
            url,
            folders: folder_chain(&folders, parent),
        });
    }
    Ok(out)
}

/// 從葉節點往上走到根，回傳由外而內的資料夾名稱。
fn folder_chain(
    folders: &std::collections::HashMap<i64, (String, i64)>,
    mut id: i64,
) -> Vec<String> {
    let mut chain = Vec::new();
    // Firefox 的根節點（id 1~5）是 menu/toolbar/tags 這類，不當標籤
    let mut guard = 0;
    while id > 5 && guard < 32 {
        let Some((name, parent)) = folders.get(&id) else {
            break;
        };
        if !name.trim().is_empty() {
            chain.push(sanitize_tag(name));
        }
        id = *parent;
        guard += 1;
    }
    chain.reverse();
    chain
}

// ---------- 共用 ----------

/// 標籤不能含空白、`#`、`,`，一律換成連字號。
pub(crate) fn sanitize_tag(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_whitespace() || c == '#' || c == ',' {
                '-'
            } else {
                c
            }
        })
        .collect();
    cleaned.chars().take(crate::model::MAX_TAG_LEN).collect()
}

/// 沒有標題的書籤用網域當標題。
fn fallback_title(title: &str, url: &str) -> String {
    let t = title.trim();
    if !t.is_empty() {
        return t.chars().take(crate::model::MAX_TITLE_LEN).collect();
    }
    url.split("//")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(url)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHROME_JSON: &str = r#"{
      "roots": {
        "bookmark_bar": {
          "name": "書籤列", "type": "folder",
          "children": [
            { "type": "url", "name": "範例", "url": "https://example.com/" },
            { "type": "folder", "name": "工作 專案",
              "children": [
                { "type": "url", "name": "文件", "url": "https://docs.example.com" },
                { "type": "folder", "name": "內部",
                  "children": [
                    { "type": "url", "name": "Wiki", "url": "https://wiki.example.com" }
                  ]
                }
              ]
            },
            { "type": "url", "name": "不是網頁", "url": "javascript:void(0)" },
            { "type": "url", "name": "", "url": "https://noname.example.com/path" }
          ]
        },
        "other": { "name": "其他書籤", "type": "folder", "children": [] }
      }
    }"#;

    #[test]
    fn parses_chromium_tree_into_flat_bookmarks() {
        let got = parse_chromium(CHROME_JSON).unwrap();
        let titles: Vec<&str> = got.iter().map(|b| b.title.as_str()).collect();
        assert_eq!(titles, ["範例", "文件", "Wiki", "noname.example.com"]);
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("inkkeep-{tag}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 一個瀏覽器的所有設定檔都要列出來，名稱用瀏覽器自己的，沒有書籤的不算。
    #[test]
    fn chromium_profiles_are_listed_with_their_names() {
        let root = temp_dir("chromium");
        for dir in ["Default", "Profile 1", "Profile 10", "Profile 2", "Guest Profile"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        // 訪客設定檔也放一個書籤檔：實際的 Chrome 就會這樣
        for dir in ["Default", "Profile 1", "Profile 10", "Guest Profile"] {
            std::fs::write(root.join(dir).join("Bookmarks"), "{}").unwrap();
        }
        std::fs::write(
            root.join("Local State"),
            r#"{"profile":{"info_cache":{"Default":{"name":"John"},"Profile 1":{"name":"工作"},"Profile 10":{"name":""},"Profile 2":{"name":"空的"}}}}"#,
        )
        .unwrap();

        let got = chromium_profiles(Source::Chrome, "Chrome", &root);
        let labels: Vec<&str> = got.iter().map(|p| p.label.as_str()).collect();
        // 訪客不在 info_cache 不列；Profile 2 登記了但沒有書籤檔也不列
        assert_eq!(labels, ["Chrome — John", "Chrome — 工作", "Chrome — Profile 10"], "空名稱退回資料夾名");
        assert_eq!(got[1].path, root.join("Profile 1").join("Bookmarks"));
        assert!(got.iter().all(|p| p.source == Source::Chrome));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn chromium_profiles_without_local_state_fall_back_to_folder_names() {
        let root = temp_dir("chromium-nols");
        for dir in ["Default", "Guest Profile"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            std::fs::write(root.join(dir).join("Bookmarks"), "{}").unwrap();
        }
        let got = chromium_profiles(Source::Edge, "Edge", &root);
        assert_eq!(got.len(), 1, "訪客設定檔要靠名稱排除");
        assert_eq!(got[0].label, "Edge — Default");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Firefox 開著時，還在 `-wal` 檔、沒寫回主檔的新書籤也要讀得到。
    #[test]
    fn firefox_bookmarks_still_in_the_wal_are_read() {
        use rusqlite::Connection;
        let root = temp_dir("firefox-wal");
        let places = root.join("places.sqlite");

        // 連線一直開著，模擬 Firefox 執行中
        let conn = Connection::open(&places).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA wal_autocheckpoint=0;
             CREATE TABLE moz_places (id INTEGER PRIMARY KEY, url TEXT);
             CREATE TABLE moz_bookmarks (
               id INTEGER PRIMARY KEY, type INTEGER, fk INTEGER, parent INTEGER, title TEXT);
             INSERT INTO moz_places VALUES (1, 'https://old.example/');
             INSERT INTO moz_bookmarks VALUES (10, 1, 1, 3, '舊的');
             PRAGMA wal_checkpoint(TRUNCATE);
             INSERT INTO moz_places VALUES (2, 'https://new.example/');
             INSERT INTO moz_bookmarks VALUES (11, 1, 2, 3, '新的');",
        )
        .unwrap();
        assert!(sidecar(&places, "-wal").exists(), "新書籤應該還在 -wal 裡");

        let got = read_firefox(&places).expect("讀得到");
        let mut titles: Vec<&str> = got.iter().map(|b| b.title.as_str()).collect();
        titles.sort();
        assert_eq!(titles, ["新的", "舊的"]);

        drop(conn);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Firefox 依 profiles.ini 列出每個設定檔。
    #[test]
    fn firefox_profiles_come_from_profiles_ini() {
        let root = temp_dir("firefox");
        let used = root.join("Profiles").join("abc.default-release");
        let unused = root.join("Profiles").join("xyz.default");
        let empty = root.join("Profiles").join("nothing");
        for d in [&used, &unused, &empty] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(used.join("places.sqlite"), b"").unwrap();
        std::fs::write(unused.join("places.sqlite"), b"").unwrap();
        std::fs::write(
            root.join("profiles.ini"),
            "[Install1]\nDefault=Profiles/abc.default-release\n\n\
             [Profile1]\nName=default\nIsRelative=1\nPath=Profiles/xyz.default\n\n\
             [Profile0]\nName=default-release\nIsRelative=1\nPath=Profiles/abc.default-release\n\n\
             [Profile2]\nName=empty\nIsRelative=1\nPath=Profiles/nothing\n\n[General]\n",
        )
        .unwrap();

        let got = firefox_profiles(&root);
        let labels: Vec<&str> = got.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(labels, ["Firefox — default", "Firefox — default-release"], "沒有 places.sqlite 的不列");
        assert_eq!(got[1].path, used.join("places.sqlite"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn folder_path_becomes_tags_and_root_is_dropped() {
        let got = parse_chromium(CHROME_JSON).unwrap();
        let wiki = got.iter().find(|b| b.title == "Wiki").unwrap();
        // 書籤列（根）不算，空白換成連字號
        assert_eq!(wiki.folders, ["工作-專案", "內部"]);

        let top = got.iter().find(|b| b.title == "範例").unwrap();
        assert!(top.folders.is_empty());
    }

    #[test]
    fn non_http_urls_are_skipped() {
        let got = parse_chromium(CHROME_JSON).unwrap();
        assert!(!got.iter().any(|b| b.url.starts_with("javascript:")));
    }

    #[test]
    fn untitled_bookmark_falls_back_to_host() {
        let got = parse_chromium(CHROME_JSON).unwrap();
        assert!(got.iter().any(|b| b.title == "noname.example.com"));
    }

    #[test]
    fn broken_json_is_reported_not_panicked() {
        assert!(matches!(
            parse_chromium("{ not json"),
            Err(ImportError::Parse(_))
        ));
    }

    fn bm(title: &str, url: &str) -> Bookmark {
        Bookmark {
            title: title.into(),
            url: url.into(),
            folders: vec![],
        }
    }

    #[test]
    fn merge_skips_urls_already_present() {
        let existing = vec![Item::new(ItemKind::Bookmark, "舊的", "https://example.com")];
        let (added, report) = merge(
            &existing,
            vec![
                bm("新的", "https://example.com/"),
                bm("另一個", "https://other.com"),
            ],
        );
        assert_eq!(report.added, 1);
        assert_eq!(report.skipped, 1);
        assert_eq!(added[0].title, "另一個");
    }

    #[test]
    fn merge_dedupes_within_the_incoming_batch() {
        let (added, report) = merge(
            &[],
            vec![bm("A", "https://same.com"), bm("B", "https://SAME.com/")],
        );
        assert_eq!(report.added, 1);
        assert_eq!(report.skipped, 1);
        assert_eq!(added.len(), 1);
    }

    #[test]
    fn merge_ignores_non_bookmark_items_when_deduping() {
        // 同樣的字串出現在片語內容裡，不該讓書籤被跳過
        let existing = vec![Item::new(ItemKind::Snippet, "片語", "https://example.com")];
        let (_, report) = merge(&existing, vec![bm("書籤", "https://example.com")]);
        assert_eq!(report.added, 1);
    }

    #[test]
    fn imported_items_are_bookmarks_with_tags() {
        let (added, _) = merge(
            &[],
            vec![Bookmark {
                title: "站".into(),
                url: "https://a.com".into(),
                folders: vec!["工作".into()],
            }],
        );
        assert_eq!(added[0].kind, ItemKind::Bookmark);
        assert_eq!(added[0].tags, ["工作"]);
    }

    #[test]
    fn tag_sanitising_strips_forbidden_characters() {
        assert_eq!(sanitize_tag(" 有 空白 "), "有-空白");
        assert_eq!(sanitize_tag("井#號"), "井-號");
        assert_eq!(sanitize_tag("逗,號"), "逗-號");
    }

    #[test]
    fn missing_file_is_reported() {
        let path = std::env::temp_dir().join("inkkeep-no-such-bookmarks");
        assert!(matches!(
            read(Source::File, Some(&path)),
            Err(ImportError::NotFound(_))
        ));
    }
}
