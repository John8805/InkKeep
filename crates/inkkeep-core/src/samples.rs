//! 範例項目。每一筆各示範一種佔位符。

use crate::model::{Item, ItemKind};

pub fn samples() -> Vec<Item> {
    let mut out = vec![
        snippet("今天日期", "${date:long}", &["範例"]),
        snippet("一週後", "${date:+7d}", &["範例"]),
        snippet(
            "感謝信",
            "${input:客戶名稱} 您好，\n\n感謝您於 ${date:long} 的來信。${cursor}\n\n敬祝 順心",
            &["範例", "mail"],
        ),
        snippet(
            "回覆狀態",
            "目前進度：${select:狀態:處理中|已完成|待確認}",
            &["範例"],
        ),
        snippet("引用剪貼簿", "> ${clipboard}", &["範例"]),
    ];
    out.push(snippet(
        "簽名",
        "--\n${input:你的名字}\n${snippet:今天日期}",
        &["範例"],
    ));
    out
}

fn snippet(title: &str, body: &str, tags: &[&str]) -> Item {
    let mut item = Item::new(ItemKind::Snippet, title, body);
    item.tags = tags.iter().map(|t| t.to_string()).collect();
    item
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::validate;
    use crate::template::{ItemResolver, Template};
    use std::collections::BTreeMap;

    struct Lib(Vec<Item>);
    impl ItemResolver for Lib {
        fn resolve(&self, title: &str) -> Option<String> {
            let key = title.to_lowercase();
            self.0
                .iter()
                .find(|i| i.title.to_lowercase() == key)
                .map(|i| i.body.clone())
        }
    }

    #[test]
    fn every_sample_passes_validation() {
        for item in samples() {
            assert!(validate(&item).is_ok(), "{} 未通過驗證", item.title);
        }
    }

    #[test]
    fn every_sample_parses_and_plans() {
        let all = samples();
        let lib = Lib(all.clone());
        for item in &all {
            let t = Template::parse(&item.body)
                .unwrap_or_else(|e| panic!("{} 解析失敗：{e}", item.title));
            t.plan(&lib)
                .unwrap_or_else(|e| panic!("{} plan 失敗：{e}", item.title));
        }
    }

    #[test]
    fn samples_cover_each_placeholder_kind() {
        let bodies: String = samples().iter().map(|i| i.body.clone()).collect();
        for needle in [
            "${date",
            "${input:",
            "${select:",
            "${clipboard}",
            "${cursor}",
            "${snippet:",
        ] {
            assert!(bodies.contains(needle), "範例沒有涵蓋 {needle}");
        }
    }

    #[test]
    fn titles_are_unique() {
        let mut titles: Vec<String> = samples().iter().map(|i| i.title.to_lowercase()).collect();
        titles.sort();
        let before = titles.len();
        titles.dedup();
        assert_eq!(
            titles.len(),
            before,
            "範例標題重複，${{snippet:}} 引用會不確定"
        );
    }

    #[test]
    fn signature_reference_resolves() {
        let all = samples();
        let lib = Lib(all.clone());
        let sig = all.iter().find(|i| i.title == "簽名").unwrap();
        let t = Template::parse(&sig.body).unwrap();
        let mut inputs = BTreeMap::new();
        inputs.insert("你的名字".to_string(), "John".to_string());
        let ctx = crate::template::RenderCtx::new(chrono::Local::now()).with_inputs(inputs);
        let out = t.render(&ctx, &lib).unwrap().text;
        assert!(out.starts_with("--\nJohn\n"), "{out}");
        assert!(out.contains('年'), "引用的日期沒展開：{out}");
    }
}
