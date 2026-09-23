//! 模板展開。

use super::{DateFormat, Node, Offset, OffsetUnit, Template, TemplateError, MAX_REFERENCE_DEPTH};
use chrono::{DateTime, Datelike, Days, Local, Months, Timelike};
use std::collections::BTreeMap;

/// 依 title 找 Snippet 的 body。
pub trait ItemResolver {
    fn resolve(&self, title: &str) -> Option<String>;
}

impl ItemResolver for () {
    fn resolve(&self, _title: &str) -> Option<String> {
        None
    }
}

impl ItemResolver for BTreeMap<String, String> {
    fn resolve(&self, title: &str) -> Option<String> {
        let key = title.to_lowercase();
        self.iter()
            .find(|(k, _)| k.to_lowercase() == key)
            .map(|(_, v)| v.clone())
    }
}

pub struct RenderCtx {
    /// 日期時間佔位符的基準時間。
    pub now: DateTime<Local>,
    pub clipboard: Option<String>,
    pub inputs: BTreeMap<String, String>,
    /// 預覽模式：缺少的輸入、剪貼簿與 cursor 以佔位標記代替。
    pub preview: bool,
}

impl RenderCtx {
    pub fn new(now: DateTime<Local>) -> Self {
        RenderCtx {
            now,
            clipboard: None,
            inputs: BTreeMap::new(),
            preview: false,
        }
    }

    pub fn with_inputs(mut self, inputs: BTreeMap<String, String>) -> Self {
        self.inputs = inputs;
        self
    }

    pub fn with_clipboard(mut self, text: impl Into<String>) -> Self {
        self.clipboard = Some(text.into());
        self
    }

    pub fn preview(mut self) -> Self {
        self.preview = true;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub text: String,
    /// cursor 之後還有幾個字元。
    pub cursor_from_end: usize,
}

const MONTHS_ZH: [&str; 12] = [
    "1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月",
];

impl Template {
    pub fn render(
        &self,
        ctx: &RenderCtx,
        resolver: &dyn ItemResolver,
    ) -> Result<Rendered, TemplateError> {
        let mut out = String::new();
        let mut cursor_at: Option<usize> = None;
        let mut stack: Vec<String> = Vec::new();
        self.render_into(ctx, resolver, &mut out, &mut cursor_at, &mut stack, 0)?;

        let cursor_from_end = match cursor_at {
            Some(chars_before) => out.chars().count() - chars_before,
            None => 0,
        };
        Ok(Rendered {
            text: out,
            cursor_from_end,
        })
    }

    fn render_into(
        &self,
        ctx: &RenderCtx,
        resolver: &dyn ItemResolver,
        out: &mut String,
        cursor_at: &mut Option<usize>,
        stack: &mut Vec<String>,
        depth: usize,
    ) -> Result<(), TemplateError> {
        if depth > MAX_REFERENCE_DEPTH {
            return Err(err("reference-too-deep"));
        }
        for node in self.nodes() {
            match node {
                Node::Text(t) => out.push_str(t),
                Node::Date { offset, format } => {
                    let d = shift(ctx.now, *offset)?;
                    out.push_str(&format_date(d, format)?);
                }
                Node::Time { format } => {
                    out.push_str(&format_time(ctx.now, format)?);
                }
                Node::DateTime { offset, format } => {
                    let d = shift(ctx.now, *offset)?;
                    // 自訂格式本身就描述了整串日期時間
                    if let DateFormat::Custom(f) = format {
                        out.push_str(&try_strftime(d, f)?);
                    } else {
                        out.push_str(&format_date(d, format)?);
                        out.push(' ');
                        out.push_str(&format_time(d, format)?);
                    }
                }
                Node::Clipboard => {
                    if ctx.preview {
                        out.push_str("[剪貼簿]");
                    } else {
                        out.push_str(ctx.clipboard.as_deref().unwrap_or(""));
                    }
                }
                Node::Cursor => {
                    if ctx.preview {
                        out.push('|');
                    } else if cursor_at.is_none() {
                        *cursor_at = Some(out.chars().count());
                    } else {
                        return Err(err("multiple-cursors"));
                    }
                }
                Node::Uuid => out.push_str(&uuid::Uuid::new_v4().to_string()),
                Node::Input { label, default } => {
                    let value = ctx.inputs.get(label).cloned().or_else(|| {
                        if ctx.preview {
                            Some(default.clone().unwrap_or_else(|| format!("[{label}]")))
                        } else {
                            default.clone()
                        }
                    });
                    out.push_str(&value.ok_or_else(|| err("missing-input"))?);
                }
                Node::Select { label, options } => {
                    let value = ctx.inputs.get(label).cloned().or_else(|| {
                        if ctx.preview {
                            options.first().cloned()
                        } else {
                            None
                        }
                    });
                    out.push_str(&value.ok_or_else(|| err("missing-input"))?);
                }
                Node::Snippet { title } => {
                    let key = title.to_lowercase();
                    if stack.contains(&key) {
                        return Err(err("reference-cycle"));
                    }
                    let body = resolver
                        .resolve(title)
                        .ok_or_else(|| err("reference-not-found"))?;
                    let inner = Template::parse(&body)?;
                    stack.push(key);
                    inner.render_into(ctx, resolver, out, cursor_at, stack, depth + 1)?;
                    stack.pop();
                }
            }
        }
        Ok(())
    }
}

fn err(message: &str) -> TemplateError {
    TemplateError {
        offset: 0,
        message: message.to_string(),
    }
}

/// 月／年偏移用日曆語意，溢位截斷到當月最後一天。
fn shift(now: DateTime<Local>, offset: Option<Offset>) -> Result<DateTime<Local>, TemplateError> {
    let Some(Offset { amount, unit }) = offset else {
        return Ok(now);
    };
    let magnitude = amount.unsigned_abs();
    let forward = amount >= 0;

    let shifted = match unit {
        OffsetUnit::Day | OffsetUnit::Week => {
            let days = magnitude
                .checked_mul(if matches!(unit, OffsetUnit::Week) {
                    7
                } else {
                    1
                })
                .ok_or_else(|| err("offset-out-of-range"))?;
            let days = Days::new(days);
            if forward {
                now.checked_add_days(days)
            } else {
                now.checked_sub_days(days)
            }
        }
        OffsetUnit::Month | OffsetUnit::Year => {
            let months = magnitude
                .checked_mul(if matches!(unit, OffsetUnit::Year) {
                    12
                } else {
                    1
                })
                .and_then(|m| u32::try_from(m).ok())
                .ok_or_else(|| err("offset-out-of-range"))?;
            let months = Months::new(months);
            if forward {
                now.checked_add_months(months)
            } else {
                now.checked_sub_months(months)
            }
        }
    };
    shifted.ok_or_else(|| err("offset-out-of-range"))
}

fn format_date(d: DateTime<Local>, format: &DateFormat) -> Result<String, TemplateError> {
    Ok(match format {
        DateFormat::Iso => d.format("%Y-%m-%d").to_string(),
        DateFormat::Long => format!("{}年{}日", d.year(), {
            let m = MONTHS_ZH[(d.month() - 1) as usize];
            format!("{m}{}", d.day())
        }),
        DateFormat::Short => format!("{}/{}/{}", d.year(), d.month(), d.day()),
        DateFormat::Hour12 => d.format("%Y-%m-%d").to_string(),
        DateFormat::Custom(f) => try_strftime(d, f)?,
    })
}

fn format_time(d: DateTime<Local>, format: &DateFormat) -> Result<String, TemplateError> {
    Ok(match format {
        DateFormat::Iso | DateFormat::Short => d.format("%H:%M").to_string(),
        DateFormat::Long | DateFormat::Hour12 => {
            let hour24 = d.hour();
            let meridiem = if hour24 < 12 { "上午" } else { "下午" };
            let hour12 = match hour24 % 12 {
                0 => 12,
                h => h,
            };
            format!("{meridiem}{hour12}:{:02}", d.minute())
        }
        DateFormat::Custom(f) => try_strftime(d, f)?,
    })
}

fn try_strftime(d: DateTime<Local>, f: &str) -> Result<String, TemplateError> {
    // 文法在 parse 階段已經驗過，這裡只擋 chrono 執行期的意外
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| d.format(f).to_string()))
        .map_err(|_| err("invalid-strftime"))
}
