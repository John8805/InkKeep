//! 模板引擎。
//!
//! 兩階段：`plan()` 遞迴收集所有需要詢問的欄位，`render()` 拿到填好的 map 直接展開。

mod parse;
mod render;

#[cfg(test)]
mod tests;

pub use render::{ItemResolver, RenderCtx, Rendered};

use serde::{Deserialize, Serialize};

/// 引用展開的深度上限。
pub const MAX_REFERENCE_DEPTH: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{message} (offset {offset})")]
pub struct TemplateError {
    /// 錯誤在原始字串中的位元組位置。
    pub offset: u32,
    /// i18n key。
    pub message: String,
    /// 錯誤牽涉的名稱：找不到或循環引用的片語標題、衝突或缺值的欄位標籤。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl TemplateError {
    pub fn new(offset: u32, message: &str) -> Self {
        TemplateError {
            offset,
            message: message.to_string(),
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffsetUnit {
    Day,
    Week,
    Month,
    Year,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offset {
    pub amount: i64,
    pub unit: OffsetUnit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DateFormat {
    Iso,
    Long,
    Short,
    Hour12,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Text(String),
    Date {
        offset: Option<Offset>,
        format: DateFormat,
    },
    Time {
        format: DateFormat,
    },
    DateTime {
        offset: Option<Offset>,
        format: DateFormat,
    },
    Clipboard,
    Cursor,
    Uuid,
    Input {
        label: String,
        default: Option<String>,
    },
    Select {
        label: String,
        options: Vec<String>,
    },
    Snippet {
        title: String,
    },
}

/// 插入前要向使用者詢問的一個欄位。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum InputField {
    Text {
        label: String,
        default: Option<String>,
    },
    Select {
        label: String,
        options: Vec<String>,
    },
}

impl InputField {
    pub fn label(&self) -> &str {
        match self {
            InputField::Text { label, .. } | InputField::Select { label, .. } => label,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsertPlan {
    /// 依出現順序、已去重。
    pub needs_inputs: Vec<InputField>,
    pub has_cursor: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    nodes: Vec<Node>,
}

impl Template {
    pub fn parse(src: &str) -> Result<Template, TemplateError> {
        Ok(Template {
            nodes: parse::parse(src)?,
        })
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// 遞迴走訪引用，收集要詢問的欄位並檢查衝突、循環、cursor 數量。
    pub fn plan(&self, resolver: &dyn ItemResolver) -> Result<InsertPlan, TemplateError> {
        let mut fields: Vec<InputField> = Vec::new();
        let mut cursors = 0usize;
        let mut stack: Vec<String> = Vec::new();
        self.collect(resolver, &mut fields, &mut cursors, &mut stack, 0)?;
        if cursors > 1 {
            return Err(TemplateError::new(0, "multiple-cursors"));
        }
        Ok(InsertPlan {
            needs_inputs: fields,
            has_cursor: cursors == 1,
        })
    }

    fn collect(
        &self,
        resolver: &dyn ItemResolver,
        fields: &mut Vec<InputField>,
        cursors: &mut usize,
        stack: &mut Vec<String>,
        depth: usize,
    ) -> Result<(), TemplateError> {
        if depth > MAX_REFERENCE_DEPTH {
            return Err(TemplateError::new(0, "reference-too-deep"));
        }
        for node in &self.nodes {
            match node {
                Node::Cursor => *cursors += 1,
                Node::Input { label, default } => push_field(
                    fields,
                    InputField::Text {
                        label: label.clone(),
                        default: default.clone(),
                    },
                )?,
                Node::Select { label, options } => push_field(
                    fields,
                    InputField::Select {
                        label: label.clone(),
                        options: options.clone(),
                    },
                )?,
                Node::Snippet { title } => {
                    let key = title.to_lowercase();
                    if stack.contains(&key) {
                        return Err(TemplateError::new(0, "reference-cycle").with_detail(title));
                    }
                    let body = resolver.resolve(title).ok_or_else(|| {
                        TemplateError::new(0, "reference-not-found").with_detail(title)
                    })?;
                    let inner = Template::parse(&body)?;
                    stack.push(key);
                    inner.collect(resolver, fields, cursors, stack, depth + 1)?;
                    stack.pop();
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// 同 label 只收一次。型別或選項不同視為模板定義錯誤；
/// `input` 之間 default 不同時取第一次出現的那個。
fn push_field(fields: &mut Vec<InputField>, incoming: InputField) -> Result<(), TemplateError> {
    if let Some(existing) = fields.iter().find(|f| f.label() == incoming.label()) {
        return match (existing, &incoming) {
            (InputField::Text { .. }, InputField::Text { .. }) => Ok(()),
            (InputField::Select { options: a, .. }, InputField::Select { options: b, .. })
                if a == b =>
            {
                Ok(())
            }
            _ => Err(TemplateError::new(0, "label-conflict").with_detail(incoming.label())),
        };
    }
    fields.push(incoming);
    Ok(())
}
