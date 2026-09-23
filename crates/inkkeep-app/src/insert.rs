//! 送出流程。
//!
//! 兩條路徑：一般項目走剪貼簿貼上，敏感項目走 `SendInput` 直接輸入。
//! 送出失敗時一般項目的內容留在剪貼簿，敏感項目不寫入剪貼簿（見 `fail`）。

use crate::commands::SendField;
use crate::error::AppError;
use crate::settings::InjectSettings;
use crate::state::AppState;
use chrono::Local;
use inkkeep_core::model::{Item, ItemKind};
use inkkeep_core::template::{ItemResolver, RenderCtx, Rendered, Template};
use inkkeep_win::{clipboard, focus, input, InputMethod, WinError};
use std::collections::BTreeMap;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

struct Resolver(Vec<Item>);

impl ItemResolver for Resolver {
    fn resolve(&self, title: &str) -> Option<String> {
        let key = title.to_lowercase();
        self.0
            .iter()
            .filter(|i| i.kind.uses_template() && i.title.to_lowercase() == key)
            .max_by_key(|i| i.updated_at)
            .map(|i| i.body.clone())
    }
}

/// 把項目展開成要送出的文字。
pub fn render_for_send(
    state: &AppState,
    id: Uuid,
    field: SendField,
    inputs: BTreeMap<String, String>,
) -> Result<Rendered, AppError> {
    let items = state.items();
    let item = items
        .iter()
        .find(|i| i.id == id)
        .ok_or_else(|| AppError::NotFound { id: id.to_string() })?;

    let plain_field = match field {
        SendField::Username => Some(&item.username),
        SendField::Url => Some(&item.url),
        SendField::Body => None,
    };
    if let Some(value) = plain_field {
        return Ok(Rendered {
            text: value.clone().unwrap_or_default(),
            cursor_from_end: 0,
        });
    }

    if !item.kind.uses_template() {
        // 密碼在保險庫裡是公鑰封起來的，要送出去之前才拆開
        let text = if item.kind == ItemKind::Password {
            crate::commands::unseal_body(state, &item.body)?
        } else {
            item.body.clone()
        };
        return Ok(Rendered {
            text,
            cursor_from_end: 0,
        });
    }

    let template = Template::parse(&item.body)?;
    let mut ctx = RenderCtx::new(Local::now()).with_inputs(inputs);
    // ${clipboard} 取的是使用者目前的剪貼簿，要在我們覆寫它之前讀
    if let Ok(Some(text)) = clipboard::read_text() {
        ctx = ctx.with_clipboard(text);
    }
    Ok(template.render(&ctx, &Resolver(items.clone()))?)
}

pub fn run(
    app: &AppHandle,
    state: &AppState,
    id: Uuid,
    field: SendField,
    inputs: BTreeMap<String, String>,
    alternate_method: bool,
) -> Result<(), AppError> {
    let settings = InjectSettings::load(app);
    let sensitive = state
        .with_vault(|v, _| v.get(id).map(|i| i.kind.is_sensitive()))
        .flatten()
        .unwrap_or(false);

    let rendered = render_for_send(state, id, field, inputs)?;

    // 先收起視窗，讓焦點有機會回到目標
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }

    let Some(token) = state.take_target() else {
        // 沒有記錄到目標視窗：不猜。一般項目放進剪貼簿，敏感項目不放
        return Err(fail(&rendered.text, WinError::Other("no-target".into()), sensitive));
    };

    if let Err(e) = focus::check_target(&token) {
        return Err(fail(&rendered.text, e, sensitive));
    }
    if let Err(e) = focus::restore_focus(&token, Duration::from_millis(settings.focus_timeout_ms)) {
        return Err(fail(&rendered.text, e, sensitive));
    }

    let outcome = if sensitive {
        send_directly(&rendered, &token, &settings, alternate_method)
    } else {
        send_via_clipboard(&rendered, &settings)
    };

    match outcome {
        Ok(()) => {
            let _ = state.with_vault_mut(|v| v.touch(id));
            let _ = app.emit("insert:done", id.to_string());
            Ok(())
        }
        Err(e) => Err(fail(&rendered.text, e, sensitive)),
    }
}

/// 一般項目：寫剪貼簿、Ctrl+V、視需要移游標，再還原原本的剪貼簿文字。
///
/// 送出去的內容會在剪貼簿上停留 `restore_delay_ms`，所以敏感項目不可走這條路。
fn send_via_clipboard(rendered: &Rendered, settings: &InjectSettings) -> Result<(), WinError> {
    // 讀不到原本的內容（剪貼簿被別的程式佔著）就不還原
    let before = clipboard::snapshot().ok();
    let our_seq = clipboard::set_text(&rendered.text)?;
    input::send_paste()?;

    if rendered.cursor_from_end > 0 {
        std::thread::sleep(Duration::from_millis(settings.paste_settle_ms));
        input::send_arrow_left(rendered.cursor_from_end as u32)?;
    }

    // 目標程式收到 Ctrl+V 後才去讀剪貼簿，還原要等它讀完。在背景等，送出不必卡住。
    // 這段期間使用者自己複製了東西，`restore` 會放棄還原。
    if let Some(before) = before {
        let delay = Duration::from_millis(settings.restore_delay_ms);
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            if let Err(e) = clipboard::restore(&before, our_seq) {
                eprintln!("還原剪貼簿失敗：{e}");
            }
        });
    }
    Ok(())
}

/// 敏感項目：逐字輸入，完全不碰剪貼簿。
fn send_directly(
    rendered: &Rendered,
    token: &inkkeep_win::FocusToken,
    settings: &InjectSettings,
    alternate_method: bool,
) -> Result<(), WinError> {
    let method = match (settings.virtual_input, alternate_method) {
        (false, false) | (true, true) => InputMethod::Unicode,
        (true, false) | (false, true) => InputMethod::Virtual,
    };
    input::send_text(
        &rendered.text,
        method,
        token,
        Duration::from_millis(settings.type_delay_ms),
    )?;
    if rendered.cursor_from_end > 0 {
        input::send_arrow_left(rendered.cursor_from_end as u32)?;
    }
    Ok(())
}

/// 送出失敗時把一般項目的內容寫進剪貼簿；敏感項目不寫。
fn fail(text: &str, error: WinError, sensitive: bool) -> AppError {
    let on_clipboard = if sensitive {
        false
    } else {
        clipboard::set_text(text).is_ok()
    };
    AppError::SendFailed {
        reason: error.to_string(),
        clipboard: on_clipboard,
    }
}
