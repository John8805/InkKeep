//! inkkeep 的 Tauri 應用進入點。
//!
//! 單一視窗三個檢視（搜尋、建立/編輯、設定），按快捷鍵顯示。

// 發行版不要跳出主控台視窗
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![cfg(windows)]

mod commands;
mod error;
mod i18n;
mod insert;
mod settings;
mod state;

use settings::Settings;
use state::AppState;
use std::time::Duration;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
use tauri_plugin_notification::NotificationExt;

fn main() {
    tauri::Builder::default()
        // single-instance 要第一個註冊。重複點執行檔時不開第二個行程，
        // 而是把已經在跑的那個叫出來——否則會多一個工作列圖示，
        // 而且第二次註冊全域快捷鍵一定失敗。
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            announce_or_reveal(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            commands::vault_state,
            commands::unlock,
            commands::unlock_secrets,
            commands::reveal_vault_key,
            commands::forget_vault_key,
            commands::create_vault,
            commands::lock,
            commands::search_items,
            commands::item_get,
            commands::reveal_password,
            commands::item_upsert,
            commands::item_delete,
            commands::template_validate,
            commands::prepare_insert,
            commands::preview,
            commands::insert,
            commands::copy_only,
            commands::open_bookmark,
            commands::generate_password,
            commands::entropy_bits,
            commands::hide_window,
            commands::set_vault_path,
            commands::settings_get,
            commands::settings_set,
            commands::change_password,
            commands::import_preview,
            commands::import_sources,
            commands::import_bookmarks,
            commands::password_import_preview,
            commands::password_import,
            commands::delete_import_file,
            commands::workspace_list,
            commands::workspace_add,
            commands::workspace_rename,
            commands::workspace_delete,
            commands::tag_list,
            commands::check_vault_path,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let settings = Settings::load(&handle);
            let vault_path = settings.resolved_vault_path(&handle);
            let state = AppState::new(vault_path.clone(), settings.lock_after_minutes);

            // 第一層金鑰在認證管理員裡就直接開，不必等使用者輸入任何東西。
            // 密碼項目仍是密文——要用到時才會問主密碼。
            if !settings.require_password_at_startup {
                if let Some(mut vault) = commands::open_with_cached_key(&vault_path) {
                    commands::prepare_workspaces(&handle, &mut vault);
                    state.unlock(vault);
                }
            }
            app.manage(state);

            setup_tray(&handle, i18n::resolve(&settings.language))?;
            register_hotkey(&handle, &settings.hotkey);
            spawn_idle_lock(handle.clone());

            if let Ok(hwnd) = handle
                .get_webview_window("main")
                .ok_or(())
                .and_then(|w| w.hwnd().map_err(|_| ()))
            {
                if let Err(e) = inkkeep_win::chrome::hide_title_bar_icon(hwnd.0 as isize) {
                    eprintln!("移除標題列圖示失敗：{e}");
                }
            }

            announce_or_reveal(&handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("Tauri 啟動失敗");
}

fn setup_tray(app: &AppHandle, lang: &str) -> tauri::Result<()> {
    let labels = i18n::tray(lang);
    let show = MenuItem::with_id(app, "show", labels.show, true, None::<&str>)?;
    let lock = MenuItem::with_id(app, "lock", labels.lock, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", labels.quit, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &lock, &quit])?;

    TrayIconBuilder::new()
        .icon(app.default_window_icon().expect("預設圖示").clone())
        .tooltip("inkkeep")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => reveal_window(app),
            "lock" => {
                if let Some(state) = app.try_state::<AppState>() {
                    state.lock_secrets();
                }
                let _ = app.emit("vault:locked", ());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

/// 啟動時註冊全域快捷鍵。失敗時發 `hotkey:conflict` 事件。
fn register_hotkey(app: &AppHandle, combo: &str) {
    if apply_hotkey(app, combo).is_err() {
        let _ = app.emit("hotkey:conflict", combo.to_string());
    }
}

/// 換成新的全域快捷鍵：先取消所有已註冊的，再註冊 `combo`。
/// 結果記進 `AppState::hotkey_conflict`：成功清掉，失敗記下這組字串。
pub(crate) fn apply_hotkey(app: &AppHandle, combo: &str) -> Result<(), String> {
    let result = parse_shortcut(combo).and_then(|shortcut| {
        let _ = app.global_shortcut().unregister_all();
        let handle = app.clone();
        app.global_shortcut()
            .on_shortcut(shortcut, move |_, _, ev| {
                if ev.state() != tauri_plugin_global_shortcut::ShortcutState::Pressed {
                    return;
                }
                reveal_window(&handle);
            })
            .map_err(|e| e.to_string())
    });
    if let Some(state) = app.try_state::<AppState>() {
        state.set_hotkey_conflict(result.is_err().then(|| combo.to_string()));
    }
    result
}

/// 解析 `Alt+Period`、`Ctrl+Shift+K`、`Super+F9` 這種字串，按鍵名稱同瀏覽器的
/// `KeyboardEvent.code`（`KeyK` 也可以寫成 `K`）。`Win`、`Meta` 視同 `Super`。
fn parse_shortcut(combo: &str) -> Result<Shortcut, String> {
    let normalized: Vec<&str> = combo
        .split('+')
        .map(|part| match part.trim().to_ascii_lowercase().as_str() {
            "win" | "meta" => "Super",
            _ => part.trim(),
        })
        .collect();
    normalized
        .join("+")
        .parse::<Shortcut>()
        .map_err(|e| e.to_string())
}

/// 點執行檔時該做什麼：需要使用者動手就開視窗，否則只發一則通知。
///
/// 「需要使用者動手」指保險庫還沒開（第一次啟動、換了裝置、或設定成每次都問主密碼），
/// 或快捷鍵註冊失敗、無法用快捷鍵叫出視窗。
fn announce_or_reveal(app: &AppHandle) {
    let needs_user = app
        .try_state::<AppState>()
        .map(|state| state.is_locked() || state.hotkey_conflict().is_some())
        .unwrap_or(true);

    if needs_user {
        reveal_window(app);
        return;
    }

    let lang = i18n::resolve(&Settings::load(app).language);
    let (title, body) = i18n::running_in_background(lang);

    // 這兩步少任何一步，show() 都會回傳 Ok 但通知不出現——見 inkkeep_win::toast
    let app_id = &app.config().identifier;
    if let Err(e) = inkkeep_win::toast::register_app_id(app_id, title) {
        eprintln!("註冊通知身分失敗：{e}");
    }
    if let Err(e) = inkkeep_win::toast::ensure_start_menu_shortcut(app_id, title) {
        eprintln!("建立開始選單捷徑失敗：{e}");
    }
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        eprintln!("通知送不出去：{e}");
    }
}

/// 叫出搜尋視窗並置中。
///
/// **只叫出、不收起**：視窗已經顯示時再呼叫，一樣是置中並取得焦點。
fn reveal_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    // 擷取前景視窗必須在顯示自己之前，否則記到的目標會變成自己。
    // 已經在前景時完全不能抓——`capture_focus` 不會排除自己的視窗，
    // 抓下去等於把送出目標換成 inkkeep 的搜尋框。
    let mut anchor = None;
    if let Some(state) = app.try_state::<AppState>() {
        if !window.is_focused().unwrap_or(false) {
            let token = inkkeep_win::focus::capture_focus().ok();
            anchor = token.as_ref().and_then(inkkeep_win::focus::center_point);
            state.set_target(token);
        }
        state.touch_activity();
    }

    center_on_active_monitor(&window, anchor);
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    let _ = app.emit("window:shown", ());
}

/// 置中到使用者正在用的那個螢幕。
///
/// `anchor` 是目標視窗的中心點。`window.center()` 置中的是「視窗**目前所在**的螢幕」，
/// 而搜尋視窗從開機起就沒離開過主螢幕，所以單用它永遠跳在主螢幕上。
///
/// `anchor` 為 `None` 或對不到螢幕時退回 `center()`。
fn center_on_active_monitor(window: &tauri::WebviewWindow, anchor: Option<(f64, f64)>) {
    let monitor = anchor.and_then(|(x, y)| window.monitor_from_point(x, y).ok().flatten());
    let (Some(monitor), Ok(size)) = (monitor, window.outer_size()) else {
        let _ = window.center();
        return;
    };

    let area = monitor.size();
    let origin = monitor.position();
    let x = origin.x + (area.width as i32 - size.width as i32) / 2;
    let y = origin.y + (area.height as i32 - size.height as i32) / 2;
    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
}

/// 閒置超時就清掉第二層金鑰。每一輪重讀 `AppState::lock_after_minutes`，
/// 設定改了下一輪就生效；0 表示停用。
///
/// 保險庫本身保持開啟，片語與書籤照常可用。
fn spawn_idle_lock(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(20));
        let Some(state) = app.try_state::<AppState>() else {
            continue;
        };
        let minutes = state.lock_after_minutes();
        if minutes == 0 || !state.secrets_unlocked() {
            continue;
        }
        if state.idle_secs() >= minutes as u64 * 60 {
            state.lock_secrets();
            let _ = app.emit("vault:locked", ());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::parse_shortcut;

    #[test]
    fn parses_browser_style_key_names() {
        for combo in ["Alt+Period", "Ctrl+Shift+K", "Ctrl+KeyK", "Win+F9", "Super+Digit1", "Alt+Space"] {
            assert!(parse_shortcut(combo).is_ok(), "{combo} 應該解析得了");
        }
        assert!(parse_shortcut("Ctrl+NoSuchKey").is_err());
        assert!(parse_shortcut("").is_err());
    }
}
