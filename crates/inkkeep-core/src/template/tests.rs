//! 模板引擎測試。時間固定在 2026-09-22 18:34:00 本地時間。

use super::*;
use chrono::{Local, NaiveDate, TimeZone};
use std::collections::BTreeMap;

fn now() -> chrono::DateTime<Local> {
    Local
        .from_local_datetime(
            &NaiveDate::from_ymd_opt(2026, 9, 22)
                .unwrap()
                .and_hms_opt(18, 34, 0)
                .unwrap(),
        )
        .single()
        .expect("固定時間不該是模稜兩可的")
}

fn ctx() -> RenderCtx {
    RenderCtx::new(now())
}

fn render(src: &str) -> String {
    Template::parse(src)
        .expect("parse")
        .render(&ctx(), &())
        .expect("render")
        .text
}

fn render_with(src: &str, c: RenderCtx, resolver: &dyn ItemResolver) -> Rendered {
    Template::parse(src)
        .expect("parse")
        .render(&c, resolver)
        .expect("render")
}

fn parse_err(src: &str) -> String {
    Template::parse(src).unwrap_err().message
}

// ---------- 文法 ----------

#[test]
fn plain_text_passes_through() {
    assert_eq!(render("你好，世界"), "你好，世界");
}

#[test]
fn escape_yields_literal_dollar_brace() {
    assert_eq!(render("$${date}"), "${date}");
    assert_eq!(render("a$${b}c"), "a${b}c");
}

#[test]
fn code_braces_are_untouched() {
    let src = "fn main() { println!(\"{}\", 1); }";
    assert_eq!(render(src), src);
}

#[test]
fn rejects_unknown_placeholder() {
    assert_eq!(parse_err("${nope}"), "unknown-placeholder");
}

#[test]
fn rejects_unterminated_placeholder() {
    assert_eq!(parse_err("${date"), "unterminated-placeholder");
}

#[test]
fn error_offset_points_at_the_placeholder() {
    let e = Template::parse("abc${nope}").unwrap_err();
    assert_eq!(e.offset, 3);
}

#[test]
fn arguments_can_escape_separators() {
    let t = Template::parse("${input:a\\:b}").unwrap();
    let plan = t.plan(&()).unwrap();
    assert_eq!(plan.needs_inputs[0].label(), "a:b");

    let t = Template::parse("${select:x:a\\|b|c}").unwrap();
    let plan = t.plan(&()).unwrap();
    match &plan.needs_inputs[0] {
        InputField::Select { options, .. } => assert_eq!(options, &["a|b", "c"]),
        other => panic!("預期 Select，拿到 {other:?}"),
    }
}

#[test]
fn rejects_invalid_escape() {
    assert_eq!(parse_err("${input:a\\qb}"), "invalid-escape");
}

// ---------- 日期 ----------

#[test]
fn date_defaults_to_iso() {
    assert_eq!(render("${date}"), "2026-09-22");
}

#[test]
fn date_long_and_short_are_zh_tw() {
    assert_eq!(render("${date::long}"), "2026年9月22日");
    assert_eq!(render("${date:long}"), "2026年9月22日");
    assert_eq!(render("${date::short}"), "2026/9/22");
}

#[test]
fn date_offsets_move_in_both_directions() {
    assert_eq!(render("${date:+7d}"), "2026-09-29");
    assert_eq!(render("${date:-7d}"), "2026-09-15");
    assert_eq!(render("${date:+1w}"), "2026-09-29");
    assert_eq!(render("${date:+1m}"), "2026-10-22");
    assert_eq!(render("${date:+1y}"), "2027-09-22");
    assert_eq!(render("${date:-1y}"), "2025-09-22");
}

#[test]
fn offset_and_format_combine() {
    assert_eq!(render("${date:+7d:long}"), "2026年9月29日");
}

#[test]
fn format_before_offset_is_rejected() {
    // 給兩個引數時，第一個位置必須是 offset
    assert_eq!(parse_err("${date:long:+7d}"), "invalid-offset");
}

#[test]
fn month_offset_clamps_to_end_of_month() {
    let t = Template::parse("${date:+1m}").unwrap();
    let jan31 = Local
        .from_local_datetime(
            &NaiveDate::from_ymd_opt(2026, 1, 31)
                .unwrap()
                .and_hms_opt(12, 0, 0)
                .unwrap(),
        )
        .single()
        .unwrap();
    let out = t.render(&RenderCtx::new(jan31), &()).unwrap().text;
    assert_eq!(out, "2026-02-28");
}

#[test]
fn time_formats() {
    assert_eq!(render("${time}"), "18:34");
    assert_eq!(render("${time:12h}"), "下午6:34");
    assert_eq!(render("${time:long}"), "下午6:34");
}

#[test]
fn morning_uses_the_am_marker() {
    let morning = Local
        .from_local_datetime(
            &NaiveDate::from_ymd_opt(2026, 9, 22)
                .unwrap()
                .and_hms_opt(9, 5, 0)
                .unwrap(),
        )
        .single()
        .unwrap();
    let t = Template::parse("${time:12h}").unwrap();
    assert_eq!(
        t.render(&RenderCtx::new(morning), &()).unwrap().text,
        "上午9:05"
    );
}

#[test]
fn datetime_joins_date_and_time_with_one_space() {
    assert_eq!(render("${datetime}"), "2026-09-22 18:34");
    assert_eq!(render("${datetime:+7d:long}"), "2026年9月29日 下午6:34");
}

#[test]
fn datetime_with_a_custom_format_renders_it_once() {
    assert_eq!(render("${datetime:%Y/%m/%d %Hh%M}"), "2026/09/22 18h34");
}

#[test]
fn custom_strftime_is_accepted() {
    assert_eq!(render("${date:%d/%m/%Y}"), "22/09/2026");
}

#[test]
fn rejects_bad_strftime() {
    assert_eq!(parse_err("${date:%Q}"), "unknown-strftime-specifier");
    assert_eq!(parse_err("${date:nonsense}"), "unknown-format");
}

// ---------- clipboard / uuid / cursor ----------

#[test]
fn clipboard_is_substituted_and_defaults_to_empty() {
    assert_eq!(render("[${clipboard}]"), "[]");
    let r = render_with("[${clipboard}]", ctx().with_clipboard("貼上的內容"), &());
    assert_eq!(r.text, "[貼上的內容]");
}

#[test]
fn uuid_is_a_v4_uuid() {
    let out = render("${uuid}");
    let parsed = uuid::Uuid::parse_str(&out).expect("合法 uuid");
    assert_eq!(parsed.get_version_num(), 4);
}

#[test]
fn cursor_reports_distance_from_end_in_chars() {
    let r = render_with("前面${cursor}後面文字", ctx(), &());
    assert_eq!(r.text, "前面後面文字");
    assert_eq!(r.cursor_from_end, 4);
}

#[test]
fn cursor_at_the_end_has_zero_distance() {
    let r = render_with("全部${cursor}", ctx(), &());
    assert_eq!(r.cursor_from_end, 0);
}

#[test]
fn two_cursors_are_rejected_by_plan() {
    let t = Template::parse("${cursor}a${cursor}").unwrap();
    assert_eq!(t.plan(&()).unwrap_err().message, "multiple-cursors");
}

// ---------- input / select ----------

#[test]
fn plan_collects_fields_in_order_and_dedupes() {
    let t = Template::parse("${input:甲}${input:乙}${input:甲}").unwrap();
    let plan = t.plan(&()).unwrap();
    let labels: Vec<&str> = plan.needs_inputs.iter().map(|f| f.label()).collect();
    assert_eq!(labels, ["甲", "乙"]);
}

#[test]
fn first_default_wins_for_repeated_input_labels() {
    let t = Template::parse("${input:甲:一}${input:甲:二}").unwrap();
    let plan = t.plan(&()).unwrap();
    match &plan.needs_inputs[0] {
        InputField::Text { default, .. } => assert_eq!(default.as_deref(), Some("一")),
        other => panic!("預期 Text，拿到 {other:?}"),
    }
}

#[test]
fn input_and_select_sharing_a_label_is_an_error() {
    let t = Template::parse("${input:甲}${select:甲:a|b}").unwrap();
    assert_eq!(t.plan(&()).unwrap_err().message, "label-conflict");
}

#[test]
fn selects_with_different_options_conflict() {
    let t = Template::parse("${select:甲:a|b}${select:甲:a|c}").unwrap();
    assert_eq!(t.plan(&()).unwrap_err().message, "label-conflict");
}

#[test]
fn select_needs_at_least_two_distinct_options() {
    assert_eq!(parse_err("${select:甲:only}"), "select-needs-two-options");
    assert_eq!(parse_err("${select:甲:a|a}"), "select-needs-two-options");
}

#[test]
fn labels_are_trimmed() {
    let t = Template::parse("${input:  甲  }").unwrap();
    assert_eq!(t.plan(&()).unwrap().needs_inputs[0].label(), "甲");
}

#[test]
fn render_uses_supplied_inputs() {
    let mut inputs = BTreeMap::new();
    inputs.insert("客戶".to_string(), "王先生".to_string());
    let r = render_with("${input:客戶} 您好", ctx().with_inputs(inputs), &());
    assert_eq!(r.text, "王先生 您好");
}

// ---------- snippet 引用 ----------

fn library() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("簽名".to_string(), "--\nJohn\n${date}".to_string()),
        ("巢狀".to_string(), "外層 ${snippet:簽名}".to_string()),
        ("要輸入".to_string(), "${input:部門}".to_string()),
    ])
}

#[test]
fn reference_is_expanded() {
    let r = render_with("末尾 ${snippet:簽名}", ctx(), &library());
    assert_eq!(r.text, "末尾 --\nJohn\n2026-09-22");
}

#[test]
fn reference_lookup_is_case_insensitive() {
    let lib = BTreeMap::from([("Sig".to_string(), "X".to_string())]);
    let r = render_with("${snippet:sig}", ctx(), &lib);
    assert_eq!(r.text, "X");
}

#[test]
fn nested_references_expand() {
    let r = render_with("${snippet:巢狀}", ctx(), &library());
    assert_eq!(r.text, "外層 --\nJohn\n2026-09-22");
}

#[test]
fn plan_collects_inputs_from_referenced_snippets() {
    let t = Template::parse("${input:甲}${snippet:要輸入}").unwrap();
    let plan = t.plan(&library()).unwrap();
    let labels: Vec<&str> = plan.needs_inputs.iter().map(|f| f.label()).collect();
    assert_eq!(labels, ["甲", "部門"]);
}

#[test]
fn self_reference_is_a_cycle() {
    let lib = BTreeMap::from([("自己".to_string(), "${snippet:自己}".to_string())]);
    let t = Template::parse("${snippet:自己}").unwrap();
    assert_eq!(t.plan(&lib).unwrap_err().message, "reference-cycle");
}

#[test]
fn mutual_reference_is_a_cycle() {
    let lib = BTreeMap::from([
        ("甲".to_string(), "${snippet:乙}".to_string()),
        ("乙".to_string(), "${snippet:甲}".to_string()),
    ]);
    let t = Template::parse("${snippet:甲}").unwrap();
    assert_eq!(t.plan(&lib).unwrap_err().message, "reference-cycle");
}

#[test]
fn deep_chain_hits_the_depth_limit() {
    let mut lib = BTreeMap::new();
    for i in 0..10 {
        lib.insert(format!("s{i}"), format!("${{snippet:s{}}}", i + 1));
    }
    lib.insert("s10".to_string(), "底".to_string());
    let t = Template::parse("${snippet:s0}").unwrap();
    assert_eq!(t.plan(&lib).unwrap_err().message, "reference-too-deep");
}

#[test]
fn missing_reference_is_reported() {
    let t = Template::parse("${snippet:不存在}").unwrap();
    let e = t.plan(&library()).unwrap_err();
    assert_eq!(e.message, "reference-not-found");
    assert_eq!(e.detail.as_deref(), Some("不存在"), "錯誤要帶出找不到的標題");

    let e = t.render(&ctx(), &library()).unwrap_err();
    assert_eq!(e.detail.as_deref(), Some("不存在"));
}

// ---------- 預覽 ----------

#[test]
fn preview_substitutes_markers_instead_of_asking() {
    let r = render_with(
        "${input:客戶}/${input:部門:業務}/${select:狀態:處理中|已完成}/${clipboard}/${cursor}",
        ctx().preview(),
        &(),
    );
    assert_eq!(r.text, "[客戶]/業務/處理中/[剪貼簿]/|");
}

#[test]
fn plan_reports_cursor_presence() {
    assert!(!Template::parse("無").unwrap().plan(&()).unwrap().has_cursor);
    assert!(
        Template::parse("${cursor}")
            .unwrap()
            .plan(&())
            .unwrap()
            .has_cursor
    );
}

#[test]
fn render_without_a_value_reports_missing_input() {
    let t = Template::parse("${input:甲}").unwrap();
    assert_eq!(t.render(&ctx(), &()).unwrap_err().message, "missing-input");
}

#[test]
fn input_default_is_used_when_no_value_given() {
    assert_eq!(render("${input:甲:預設值}"), "預設值");
}
