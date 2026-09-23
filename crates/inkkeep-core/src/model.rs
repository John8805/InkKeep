//! 資料模型與驗證規則。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_TITLE_LEN: usize = 200;
pub const MAX_BODY_BYTES: usize = 64 * 1024;
pub const MAX_TAG_LEN: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    Snippet,
    Bookmark,
    Password,
}

impl ItemKind {
    /// 只有 Snippet 會走模板展開。
    pub fn uses_template(self) -> bool {
        matches!(self, ItemKind::Snippet)
    }

    /// 敏感項目要直接輸入，不能經過剪貼簿。
    pub fn is_sensitive(self) -> bool {
        matches!(self, ItemKind::Password)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub id: Uuid,
    pub title: String,
    pub kind: ItemKind,
    /// Snippet: 模板文字；Bookmark: URL；Password: 密碼
    pub body: String,
    /// 只有 Password 用
    pub username: Option<String>,
    /// 這組帳密屬於哪個網站。**只有 Password 用**——Bookmark 的網址是它的 `body`。
    pub url: Option<String>,
    /// 屬於哪個工作區（kdbx 群組的 UUID）。`None` 是放在根群組、還沒分配的項目。
    pub workspace: Option<Uuid>,
    pub tags: Vec<String>,
    pub use_count: u32,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Item {
    pub fn new(kind: ItemKind, title: impl Into<String>, body: impl Into<String>) -> Self {
        let now = Utc::now();
        Item {
            id: Uuid::new_v4(),
            title: title.into(),
            kind,
            body: body.into(),
            username: None,
            url: None,
            workspace: None,
            tags: Vec::new(),
            use_count: 0,
            last_used_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// 主要欄位，即 `body`。
    pub fn primary_field(&self) -> &str {
        &self.body
    }
}

/// 驗證失敗的欄位與 i18n key。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
}

impl ValidationError {
    fn new(field: &str, message: &str) -> Self {
        ValidationError {
            field: field.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

impl std::error::Error for ValidationError {}

/// 檢查項目欄位規則，模板文法另外由 `template::parse` 檢查。
pub fn validate(item: &Item) -> Result<(), Vec<ValidationError>> {
    let mut errs = Vec::new();

    let title = item.title.trim();
    if title.is_empty() {
        errs.push(ValidationError::new("title", "title-empty"));
    } else if title.chars().count() > MAX_TITLE_LEN {
        errs.push(ValidationError::new("title", "title-too-long"));
    }

    if item.body.len() > MAX_BODY_BYTES {
        errs.push(ValidationError::new("body", "body-too-large"));
    }

    // 書籤要有網址、密碼要有密碼：kdbx 是靠「哪個欄位非空」
    // 回推類型的（見 vault::detect_kind）。body 空白的密碼項目存進去之後
    // Password 欄位是空的，讀回來就變成片語——存檔後的回讀驗證必然不過。
    if item.kind == ItemKind::Bookmark && url::Url::parse(item.body.trim()).is_err() {
        errs.push(ValidationError::new("body", "invalid-url"));
    }

    if item.kind == ItemKind::Password && item.body.is_empty() {
        errs.push(ValidationError::new("body", "password-empty"));
    }

    for tag in &item.tags {
        let t = tag.trim();
        if t.is_empty() {
            errs.push(ValidationError::new("tags", "tag-empty"));
        } else if t.chars().count() > MAX_TAG_LEN {
            errs.push(ValidationError::new("tags", "tag-too-long"));
        } else if t.chars().any(|c| c.is_whitespace() || c == '#' || c == ',') {
            errs.push(ValidationError::new("tags", "tag-invalid-char"));
        }
    }

    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 空白密碼過不了驗證。
    #[test]
    fn a_password_item_must_have_a_password() {
        let item = Item::new(ItemKind::Password, "GitHub", "");
        let errs = validate(&item).expect_err("空白密碼應該被擋下來");
        assert!(
            errs.iter().any(|e| e.field == "body" && e.message == "password-empty"),
            "拿到 {errs:?}"
        );

        assert!(validate(&Item::new(ItemKind::Password, "GitHub", "hunter2")).is_ok());
    }

    /// 片語可以是空的：空 body 讀回來仍是片語。
    #[test]
    fn an_empty_snippet_is_allowed() {
        assert!(validate(&Item::new(ItemKind::Snippet, "待補", "")).is_ok());
    }

    fn snippet(title: &str, body: &str) -> Item {
        Item::new(ItemKind::Snippet, title, body)
    }

    #[test]
    fn accepts_a_plain_snippet() {
        assert!(validate(&snippet("問候", "你好")).is_ok());
    }

    #[test]
    fn rejects_blank_title() {
        let errs = validate(&snippet("   ", "x")).unwrap_err();
        assert_eq!(errs[0].message, "title-empty");
    }

    #[test]
    fn title_length_counts_chars_not_bytes() {
        let title: String = "中".repeat(MAX_TITLE_LEN);
        assert!(validate(&snippet(&title, "x")).is_ok());

        let too_long: String = "中".repeat(MAX_TITLE_LEN + 1);
        let errs = validate(&snippet(&too_long, "x")).unwrap_err();
        assert_eq!(errs[0].message, "title-too-long");
    }

    #[test]
    fn rejects_oversized_body() {
        let body = "a".repeat(MAX_BODY_BYTES + 1);
        let errs = validate(&snippet("t", &body)).unwrap_err();
        assert_eq!(errs[0].message, "body-too-large");
    }

    #[test]
    fn bookmark_body_must_parse_as_url() {
        let ok = Item::new(ItemKind::Bookmark, "站", "https://example.com/a?b=1");
        assert!(validate(&ok).is_ok());

        let bad = Item::new(ItemKind::Bookmark, "站", "不是網址");
        let errs = validate(&bad).unwrap_err();
        assert_eq!(errs[0].message, "invalid-url");
    }

    #[test]
    fn snippet_body_is_not_url_checked() {
        assert!(validate(&snippet("t", "不是網址")).is_ok());
    }

    #[test]
    fn rejects_tags_with_separators() {
        for bad in ["有 空白", "井#號", "逗,號"] {
            let mut item = snippet("t", "x");
            item.tags = vec![bad.into()];
            let errs = validate(&item).unwrap_err();
            assert_eq!(errs[0].message, "tag-invalid-char", "tag {bad:?}");
        }
    }

    #[test]
    fn reports_every_broken_field_at_once() {
        let mut item = snippet("", "x");
        item.tags = vec!["a b".into()];
        let errs = validate(&item).unwrap_err();
        assert_eq!(errs.len(), 2);
    }

    #[test]
    fn kind_drives_template_and_sensitivity() {
        assert!(ItemKind::Snippet.uses_template());
        assert!(!ItemKind::Bookmark.uses_template());
        assert!(ItemKind::Password.is_sensitive());
        assert!(!ItemKind::Snippet.is_sensitive());
    }
}
