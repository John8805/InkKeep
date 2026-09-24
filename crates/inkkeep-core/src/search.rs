//! 搜尋與排序。

use crate::model::{Item, ItemKind};
use unicode_normalization::UnicodeNormalization;

pub const DEFAULT_LIMIT: usize = 200;

/// NFC 正規化後轉小寫。查詢與被查的內容都要走這一步。
pub fn normalize(s: &str) -> String {
    s.nfc().collect::<String>().to_lowercase()
}

/// 一筆項目的預先正規化快取。
#[derive(Debug, Clone)]
struct Indexed {
    idx: usize,
    title: String,
    /// Password 的 body 不入索引，避免用搜尋反推密碼內容。
    haystack: String,
    tags: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Index {
    entries: Vec<Indexed>,
}

impl Index {
    pub fn build(items: &[Item]) -> Self {
        let entries = items
            .iter()
            .enumerate()
            .map(|(idx, item)| {
                let title = normalize(&item.title);
                let mut haystack = title.clone();
                match item.kind {
                    ItemKind::Password => {
                        if let Some(u) = &item.url {
                            haystack.push('\n');
                            haystack.push_str(&normalize(u));
                        }
                        if let Some(u) = &item.username {
                            haystack.push('\n');
                            haystack.push_str(&normalize(u));
                        }
                    }
                    _ => {
                        haystack.push('\n');
                        haystack.push_str(&normalize(&item.body));
                    }
                }
                Indexed {
                    idx,
                    title,
                    haystack,
                    tags: item.tags.iter().map(|t| normalize(t)).collect(),
                }
            })
            .collect();
        Index { entries }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// 拆出的查詢：空白切開的每個 token 都要出現在內文裡。`#` 是普通字元。
struct Query {
    terms: Vec<String>,
    /// 原始查詢正規化後的整串，用來判斷 title 完全相符
    whole: String,
}

fn parse_query(raw: &str) -> Query {
    let normalized = normalize(raw);
    let terms = normalized.split_whitespace().map(str::to_string).collect();
    Query {
        terms,
        whole: normalized.trim().to_string(),
    }
}

/// 回傳符合的項目在 `items` 中的索引，已排序。
///
/// 排序規則：title 完全等於查詢字串的排最前；其餘依 `use_count` 降序，
/// 同值依 title 字典序。空查詢回傳全部，同一套排序。
/// `tags` 裡的每個標籤項目都要有（完全相符，不分大小寫）。
/// `kind`、`workspace` 是 `None` 就不依該欄過濾。過濾在排名與截斷之前，
/// `limit` 算的是過濾後的筆數。
pub fn search(
    index: &Index,
    items: &[Item],
    raw_query: &str,
    tags: &[String],
    kind: Option<ItemKind>,
    workspace: Option<uuid::Uuid>,
    limit: usize,
) -> Vec<usize> {
    let q = parse_query(raw_query);
    let wanted_tags: Vec<String> = tags.iter().map(|t| normalize(t)).collect();

    let mut hits: Vec<(&Indexed, bool)> = index
        .entries
        .iter()
        .filter(|e| match kind {
            Some(want) => items[e.idx].kind == want,
            None => true,
        })
        .filter(|e| match workspace {
            Some(want) => items[e.idx].workspace == Some(want),
            None => true,
        })
        .filter(|e| wanted_tags.iter().all(|want| e.tags.contains(want)))
        .filter(|e| q.terms.iter().all(|term| e.haystack.contains(term)))
        .map(|e| {
            let exact = !q.whole.is_empty() && e.title == q.whole;
            (e, exact)
        })
        .collect();

    hits.sort_by(|a, b| {
        b.1.cmp(&a.1) // 完全相符優先
            .then_with(|| items[b.0.idx].use_count.cmp(&items[a.0.idx].use_count))
            .then_with(|| a.0.title.cmp(&b.0.title))
    });

    hits.into_iter().take(limit).map(|(e, _)| e.idx).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Item;

    fn item(kind: ItemKind, title: &str, body: &str, use_count: u32) -> Item {
        let mut i = Item::new(kind, title, body);
        i.use_count = use_count;
        i
    }

    fn fixture() -> Vec<Item> {
        vec![
            item(ItemKind::Snippet, "問候語", "你好，很高興認識你", 5),
            item(ItemKind::Snippet, "簽名", "敬祝 順心\n John", 12),
            item(ItemKind::Bookmark, "文件", "https://docs.example.com", 3),
            item(ItemKind::Snippet, "感謝", "感謝您的來信", 12),
        ]
    }

    fn titles(items: &[Item], idxs: &[usize]) -> Vec<String> {
        idxs.iter().map(|i| items[*i].title.clone()).collect()
    }

    #[test]
    fn kind_filter_narrows_before_ranking() {
        let items = fixture();
        let idx = Index::build(&items);

        let books = search(&idx, &items, "", &[], Some(ItemKind::Bookmark), None, DEFAULT_LIMIT);
        assert_eq!(titles(&items, &books), ["文件"]);

        let snippets = search(&idx, &items, "", &[], Some(ItemKind::Snippet), None, DEFAULT_LIMIT);
        assert_eq!(titles(&items, &snippets), ["感謝", "簽名", "問候語"]);

        // 過濾要在截斷之前：先取 limit 再過濾的話這裡會是空的，
        // 因為排名最前的兩筆都是片語
        let one_book = search(&idx, &items, "", &[], Some(ItemKind::Bookmark), None, 1);
        assert_eq!(titles(&items, &one_book), ["文件"]);
    }

    #[test]
    fn workspace_filter_narrows_the_results() {
        let mut items = fixture();
        let work = uuid::Uuid::new_v4();
        let home = uuid::Uuid::new_v4();
        for (i, item) in items.iter_mut().enumerate() {
            item.workspace = Some(if i % 2 == 0 { work } else { home });
        }
        let idx = Index::build(&items);

        let at_work = search(&idx, &items, "", &[], None, Some(work), DEFAULT_LIMIT);
        assert!(!at_work.is_empty());
        assert!(at_work.iter().all(|&i| items[i].workspace == Some(work)));

        let all = search(&idx, &items, "", &[], None, None, DEFAULT_LIMIT);
        assert_eq!(all.len(), items.len(), "None 是全部工作區");

        // 工作區跟類別可以一起用
        let both = search(&idx, &items, "", &[], Some(ItemKind::Snippet), Some(home), DEFAULT_LIMIT);
        assert!(both
            .iter()
            .all(|&i| items[i].kind == ItemKind::Snippet && items[i].workspace == Some(home)));
    }

    #[test]
    fn kind_filter_combines_with_the_query() {
        let items = fixture();
        let idx = Index::build(&items);
        assert_eq!(
            search(&idx, &items, "感謝", &[], Some(ItemKind::Snippet), None, DEFAULT_LIMIT).len(),
            1
        );
        assert_eq!(
            search(&idx, &items, "感謝", &[], Some(ItemKind::Bookmark), None, DEFAULT_LIMIT).len(),
            0
        );
    }

    #[test]
    fn empty_query_returns_all_by_use_count_then_title() {
        let items = fixture();
        let idx = Index::build(&items);
        let got = search(&idx, &items, "", &[], None, None, DEFAULT_LIMIT);
        // use_count 12 的兩筆在前，彼此依 title 字典序；「感謝」在「簽名」之前
        assert_eq!(titles(&items, &got), ["感謝", "簽名", "問候語", "文件"]);
    }

    #[test]
    fn matches_against_body() {
        let items = fixture();
        let idx = Index::build(&items);
        let got = search(&idx, &items, "很高興", &[], None, None, DEFAULT_LIMIT);
        assert_eq!(titles(&items, &got), ["問候語"]);
    }

    #[test]
    fn every_token_must_match() {
        let items = fixture();
        let idx = Index::build(&items);
        assert_eq!(search(&idx, &items, "感謝 來信", &[], None, None, DEFAULT_LIMIT).len(), 1);
        assert_eq!(search(&idx, &items, "感謝 不存在", &[], None, None, DEFAULT_LIMIT).len(), 0);
    }

    #[test]
    fn exact_title_match_is_hoisted() {
        let mut items = fixture();
        // 讓一筆 use_count 最低但 title 完全相符
        items.push(item(ItemKind::Snippet, "感謝", "另一筆", 0));
        let idx = Index::build(&items);
        let got = search(&idx, &items, "感謝", &[], None, None, DEFAULT_LIMIT);
        // 兩筆 title 都完全相符，排在最前；其餘依 use_count
        assert_eq!(items[got[0]].title, "感謝");
        assert_eq!(items[got[1]].title, "感謝");
    }

    #[test]
    fn tags_filter_and_combine_with_text() {
        let mut items = fixture();
        items[0].tags = vec!["工作".into()];
        items[1].tags = vec!["工作".into(), "私人".into()];
        let idx = Index::build(&items);
        let work = ["工作".to_string()];

        let got = search(&idx, &items, "", &work, None, None, DEFAULT_LIMIT);
        assert_eq!(titles(&items, &got), ["簽名", "問候語"]);

        let got = search(&idx, &items, "你好", &work, None, None, DEFAULT_LIMIT);
        assert_eq!(titles(&items, &got), ["問候語"]);
    }

    #[test]
    fn tags_match_whole_names_only() {
        let mut items = fixture();
        items[0].tags = vec!["工作日誌".into()];
        let idx = Index::build(&items);
        let work = ["工作".to_string()];
        assert_eq!(search(&idx, &items, "", &work, None, None, DEFAULT_LIMIT).len(), 0);
    }

    /// 查詢裡的 `#` 是普通字元，`#include` 這類內容照樣搜得到
    #[test]
    fn hash_in_query_is_plain_text() {
        let mut items = fixture();
        items[0].body = "#include <stdio.h>".into();
        items[1].tags = vec!["include".into()];
        let idx = Index::build(&items);
        let got = search(&idx, &items, "#include", &[], None, None, DEFAULT_LIMIT);
        assert_eq!(got, [0]);
    }

    #[test]
    fn password_body_is_not_searchable() {
        let mut pw = item(ItemKind::Password, "GitHub", "hunter2secret", 1);
        pw.username = Some("john".into());
        let items = vec![pw];
        let idx = Index::build(&items);

        assert_eq!(
            search(&idx, &items, "hunter2secret", &[], None, None, DEFAULT_LIMIT).len(),
            0
        );
        assert_eq!(search(&idx, &items, "github", &[], None, None, DEFAULT_LIMIT).len(), 1);
        assert_eq!(search(&idx, &items, "john", &[], None, None, DEFAULT_LIMIT).len(), 1);
    }

    #[test]
    fn matching_is_case_insensitive_and_nfc_normalized() {
        let items = vec![item(ItemKind::Snippet, "Hello", "World", 0)];
        let idx = Index::build(&items);
        assert_eq!(search(&idx, &items, "hello", &[], None, None, DEFAULT_LIMIT).len(), 1);
        assert_eq!(search(&idx, &items, "WORLD", &[], None, None, DEFAULT_LIMIT).len(), 1);

        // NFD 的「が」應該要match NFC 的「が」
        let items = vec![item(ItemKind::Snippet, "\u{304C}", "x", 0)];
        let idx = Index::build(&items);
        assert_eq!(
            search(&idx, &items, "\u{304B}\u{3099}", &[], None, None, DEFAULT_LIMIT).len(),
            1
        );
    }

    #[test]
    fn limit_is_applied_after_sorting() {
        let items = fixture();
        let idx = Index::build(&items);
        let got = search(&idx, &items, "", &[], None, None, 2);
        assert_eq!(titles(&items, &got), ["感謝", "簽名"]);
    }

    #[test]
    fn thousand_items_stay_fast() {
        let items: Vec<Item> = (0..1000)
            .map(|i| {
                item(
                    ItemKind::Snippet,
                    &format!("項目 {i}"),
                    &format!("內容 {i} 的說明文字"),
                    i % 50,
                )
            })
            .collect();
        let idx = Index::build(&items);

        let start = std::time::Instant::now();
        let got = search(&idx, &items, "說明", &[], None, None, DEFAULT_LIMIT);
        let elapsed = start.elapsed();

        assert_eq!(got.len(), DEFAULT_LIMIT);
        // debug build 會慢很多，這裡放寬到 200 ms；release 的目標是 5 ms
        assert!(elapsed.as_millis() < 200, "搜尋花了 {elapsed:?}");
    }
}
