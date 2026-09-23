//! 模板文法解析。
//!
//! ```text
//! template   := (text | escape | placeholder)*
//! escape     := "$${"                       -- 輸出字面 "${"
//! placeholder:= "${" name (":" arg)* "}"
//! name       := [a-z]+
//! arg        := (argchar | argescape)*
//! argchar    := 任何字元，除了 ":" "|" "}" "\"
//! argescape  := "\:" | "\|" | "\}" | "\\"
//! ```

use super::{DateFormat, Node, Offset, OffsetUnit, TemplateError};

/// 佔位符名稱與引數上限。
fn arg_limit(name: &str) -> Option<usize> {
    Some(match name {
        "date" | "datetime" => 2,
        "time" => 1,
        "clipboard" | "cursor" | "uuid" => 0,
        "input" => 2,
        "select" => 2,
        "snippet" => 1,
        _ => return None,
    })
}

pub fn parse(src: &str) -> Result<Vec<Node>, TemplateError> {
    Parser::new(src).run()
}

struct Parser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    out: Vec<Node>,
    text: String,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Parser {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            out: Vec::new(),
            text: String::new(),
        }
    }

    fn flush_text(&mut self) {
        if !self.text.is_empty() {
            self.out.push(Node::Text(std::mem::take(&mut self.text)));
        }
    }

    fn err(&self, offset: usize, message: &str) -> TemplateError {
        TemplateError {
            offset: offset as u32,
            message: message.to_string(),
        }
    }

    fn run(mut self) -> Result<Vec<Node>, TemplateError> {
        while self.pos < self.bytes.len() {
            if self.src[self.pos..].starts_with("$${") {
                self.text.push_str("${");
                self.pos += 3;
                continue;
            }
            if self.src[self.pos..].starts_with("${") {
                let start = self.pos;
                self.pos += 2;
                let node = self.placeholder(start)?;
                self.flush_text();
                self.out.push(node);
                continue;
            }
            let ch = self.src[self.pos..].chars().next().expect("in bounds");
            self.text.push(ch);
            self.pos += ch.len_utf8();
        }
        self.flush_text();
        Ok(self.out)
    }

    /// `self.pos` 已在 `${` 之後。
    fn placeholder(&mut self, start: usize) -> Result<Node, TemplateError> {
        let name_start = self.pos;
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_lowercase() {
            self.pos += 1;
        }
        let name = &self.src[name_start..self.pos];
        if name.is_empty() {
            return Err(self.err(start, "empty-placeholder-name"));
        }
        let Some(limit) = arg_limit(name) else {
            return Err(self.err(start, "unknown-placeholder"));
        };

        // 每個引數是「在未逃逸的 | 處切開的片段」。先切再還原，
        // `\|` 還原出的 `|` 是片段內的字面字元，不當分隔符。
        let mut args: Vec<Vec<String>> = Vec::new();
        loop {
            match self.bytes.get(self.pos) {
                None => return Err(self.err(start, "unterminated-placeholder")),
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                Some(b':') => {
                    self.pos += 1;
                    args.push(self.arg(start)?);
                }
                Some(_) => return Err(self.err(self.pos, "expected-colon-or-brace")),
            }
        }

        if args.len() > limit {
            return Err(self.err(start, "too-many-arguments"));
        }

        build(name, &args, start).map_err(|message| self.err(start, &message))
    }

    /// 讀一個引數，到下一個未逃逸的 `:` 或 `}` 為止。
    /// 回傳在未逃逸的 `|` 處切開的片段，每段已還原逃逸。
    fn arg(&mut self, start: usize) -> Result<Vec<String>, TemplateError> {
        let mut parts: Vec<String> = Vec::new();
        let mut out = String::new();
        loop {
            let Some(&b) = self.bytes.get(self.pos) else {
                return Err(self.err(start, "unterminated-placeholder"));
            };
            match b {
                b':' | b'}' => {
                    parts.push(out);
                    return Ok(parts);
                }
                b'|' => {
                    parts.push(std::mem::take(&mut out));
                    self.pos += 1;
                }
                b'\\' => {
                    let Some(&next) = self.bytes.get(self.pos + 1) else {
                        return Err(self.err(self.pos, "dangling-escape"));
                    };
                    match next {
                        b':' | b'|' | b'}' | b'\\' => {
                            out.push(next as char);
                            self.pos += 2;
                        }
                        _ => return Err(self.err(self.pos, "invalid-escape")),
                    }
                }
                _ => {
                    let ch = self.src[self.pos..].chars().next().expect("in bounds");
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }
}

/// `${date:...}` / `${datetime:...}` 的引數判定：第一個引數符合 offset 文法
/// 就當 offset，否則當 format；第二個引數只能是 format。
fn split_date_args(args: &[Vec<String>]) -> Result<(Option<Offset>, DateFormat), String> {
    let args: Vec<String> = args.iter().map(|a| joined(a)).collect();
    match args.as_slice() {
        [] => Ok((None, DateFormat::Iso)),
        [one] => {
            if one.is_empty() {
                Ok((None, DateFormat::Iso))
            } else if let Some(off) = Offset::parse(one) {
                Ok((Some(off), DateFormat::Iso))
            } else {
                Ok((None, DateFormat::parse(one)?))
            }
        }
        [first, second] => {
            let off = if first.is_empty() {
                None
            } else {
                Some(Offset::parse(first).ok_or_else(|| "invalid-offset".to_string())?)
            };
            Ok((off, DateFormat::parse(second)?))
        }
        _ => Err("too-many-arguments".into()),
    }
}

/// 把片段接回單一字串。`|` 在 select 以外沒有特殊意義。
fn joined(arg: &[String]) -> String {
    arg.join("|")
}

fn build(name: &str, args: &[Vec<String>], _start: usize) -> Result<Node, String> {
    match name {
        "date" => {
            let (offset, format) = split_date_args(args)?;
            Ok(Node::Date { offset, format })
        }
        "datetime" => {
            let (offset, format) = split_date_args(args)?;
            Ok(Node::DateTime { offset, format })
        }
        "time" => {
            let format = match args {
                [] => DateFormat::Iso,
                [one] => {
                    let one = joined(one);
                    if one.is_empty() {
                        DateFormat::Iso
                    } else {
                        DateFormat::parse(&one)?
                    }
                }
                _ => return Err("too-many-arguments".into()),
            };
            Ok(Node::Time { format })
        }
        "clipboard" => Ok(Node::Clipboard),
        "cursor" => Ok(Node::Cursor),
        "uuid" => Ok(Node::Uuid),
        "input" => {
            let label = args.first().map(|a| joined(a)).unwrap_or_default();
            let label = label.trim();
            if label.is_empty() {
                return Err("input-needs-label".into());
            }
            Ok(Node::Input {
                label: label.to_string(),
                default: args.get(1).map(|a| joined(a)),
            })
        }
        "select" => {
            let label = args.first().map(|a| joined(a)).unwrap_or_default();
            let label = label.trim();
            if label.is_empty() {
                return Err("select-needs-label".into());
            }
            let raw = args.get(1).ok_or("select-needs-options")?;
            let mut options: Vec<String> = Vec::new();
            for opt in raw {
                let opt = opt.trim();
                if opt.is_empty() || options.iter().any(|o| o == opt) {
                    continue;
                }
                options.push(opt.to_string());
            }
            if options.len() < 2 {
                return Err("select-needs-two-options".into());
            }
            Ok(Node::Select {
                label: label.to_string(),
                options,
            })
        }
        "snippet" => {
            let title = args.first().map(|a| joined(a)).unwrap_or_default();
            let title = title.trim();
            if title.is_empty() {
                return Err("snippet-needs-title".into());
            }
            Ok(Node::Snippet {
                title: title.to_string(),
            })
        }
        _ => Err("unknown-placeholder".into()),
    }
}

impl Offset {
    /// `^[+-]\d+[dwmy]$`
    fn parse(s: &str) -> Option<Offset> {
        let bytes = s.as_bytes();
        if bytes.len() < 3 {
            return None;
        }
        let negative = match bytes[0] {
            b'+' => false,
            b'-' => true,
            _ => return None,
        };
        let unit = match bytes[bytes.len() - 1] {
            b'd' => OffsetUnit::Day,
            b'w' => OffsetUnit::Week,
            b'm' => OffsetUnit::Month,
            b'y' => OffsetUnit::Year,
            _ => return None,
        };
        let digits = &s[1..s.len() - 1];
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let n: i64 = digits.parse().ok()?;
        Some(Offset {
            amount: if negative { -n } else { n },
            unit,
        })
    }
}

impl DateFormat {
    fn parse(s: &str) -> Result<DateFormat, String> {
        Ok(match s {
            "iso" => DateFormat::Iso,
            "long" => DateFormat::Long,
            "short" => DateFormat::Short,
            "12h" => DateFormat::Hour12,
            other => {
                validate_strftime(other)?;
                DateFormat::Custom(other.to_string())
            }
        })
    }
}

/// 只檢查 `%` 後面是不是 chrono 認得的指示符。
fn validate_strftime(s: &str) -> Result<(), String> {
    const SPECIFIERS: &str = "aAbBcCdDeFgGhHIjklmMnpPqrRsStTuUvVwWxXyYzZ%";
    let mut it = s.chars().peekable();
    let mut saw_specifier = false;
    while let Some(c) = it.next() {
        if c != '%' {
            continue;
        }
        // 接受 %-d、%_d、%0d 這類補零修飾
        let mut next = it.next().ok_or("dangling-percent")?;
        if matches!(next, '-' | '_' | '0') {
            next = it.next().ok_or("dangling-percent")?;
        }
        if !SPECIFIERS.contains(next) {
            return Err("unknown-strftime-specifier".into());
        }
        saw_specifier = true;
    }
    if saw_specifier {
        Ok(())
    } else {
        Err("unknown-format".into())
    }
}
