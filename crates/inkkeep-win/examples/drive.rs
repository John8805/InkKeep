//! UI 測試驅動：用 inkkeep-win 自己的輸入原語操作視窗。
//!
//! 文字走 `KEYEVENTF_UNICODE` 路徑，繞過輸入法轉換。
//!
//! 用法：drive <視窗標題> <動作>...
//!   text:<文字>   逐字輸入
//!   key:<名稱>    送一個按鍵（enter/esc/down/up/tab）
//!   wait:<毫秒>   等待
//!   ctrl:<字母>      送 Ctrl+<字母>（例如 ctrl:a、ctrl:c）
//!   alt:<鍵>        送 Alt+<鍵>。鍵可以是字母或 `.`（句點）
//!   ctrlalt:<字母>   送 Ctrl+Alt+<字母>。用來觸發全域快捷鍵，
//!                    必須由本行程送出——外部工具送完就結束，
//!                    焦點會跳回它的主控台，被測程式擷取到的前景就錯了
//!   retarget:<標題>  換一個目標視窗（不改前景）。按下快捷鍵叫出被測視窗後要用
//!   focus         重新把視窗帶到前景
//!   read          讀目前前景視窗標題
#![cfg(windows)]

use inkkeep_win::{focus, input, InputMethod};
use std::time::Duration;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_MENU, VK_OEM_COMMA, VK_OEM_PERIOD, VK_RETURN, VK_TAB, VK_UP,
};

fn press(vk: VIRTUAL_KEY) {
    let mk = |flags: u32| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS(flags),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let events = [mk(0), mk(KEYEVENTF_KEYUP.0)];
    unsafe {
        SendInput(&events, std::mem::size_of::<INPUT>() as i32);
    }
}

/// 按住若干修飾鍵再敲一個鍵。
fn send_with_modifiers(key: char, modifiers: &[VIRTUAL_KEY]) {
    // 句點等符號的虛擬鍵碼不等於字元碼，要另外對應
    let vk = match key {
        '.' => VK_OEM_PERIOD,
        ',' => VK_OEM_COMMA,
        c => VIRTUAL_KEY(c.to_ascii_uppercase() as u16),
    };
    let mk = |k: VIRTUAL_KEY, flags: u32| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: k,
                wScan: 0,
                dwFlags: windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS(flags),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let mut events: Vec<INPUT> = modifiers.iter().map(|m| mk(*m, 0)).collect();
    events.push(mk(vk, 0));
    events.push(mk(vk, KEYEVENTF_KEYUP.0));
    for m in modifiers.iter().rev() {
        events.push(mk(*m, KEYEVENTF_KEYUP.0));
    }
    unsafe {
        SendInput(&events, std::mem::size_of::<INPUT>() as i32);
    }
}

fn send_ctrl_alt(letter: char) {
    let vk = VIRTUAL_KEY(letter.to_ascii_uppercase() as u16);
    let mk = |k: VIRTUAL_KEY, flags: u32| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: k,
                wScan: 0,
                dwFlags: windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS(flags),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let events = [
        mk(VK_CONTROL, 0),
        mk(VK_MENU, 0),
        mk(vk, 0),
        mk(vk, KEYEVENTF_KEYUP.0),
        mk(VK_MENU, KEYEVENTF_KEYUP.0),
        mk(VK_CONTROL, KEYEVENTF_KEYUP.0),
    ];
    unsafe {
        SendInput(&events, std::mem::size_of::<INPUT>() as i32);
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(title) = args.next() else {
        eprintln!("用法：drive <視窗標題> <動作>...");
        std::process::exit(2);
    };

    let Some(mut token) = focus::find_window(&title) else {
        eprintln!("找不到標題含 {title:?} 的視窗");
        std::process::exit(1);
    };
    if let Err(e) = focus::restore_focus(&token, Duration::from_millis(1500)) {
        eprintln!("切換焦點失敗：{e}");
        std::process::exit(1);
    }
    println!("focus -> {:?}", token.title());

    for action in args {
        let (verb, arg) = action.split_once(':').unwrap_or((action.as_str(), ""));
        match verb {
            "text" => {
                if let Err(e) =
                    input::send_text(arg, InputMethod::Unicode, &token, Duration::from_millis(3))
                {
                    eprintln!("輸入中止：{e}");
                    std::process::exit(1);
                }
            }
            "key" => match arg {
                "enter" => press(VK_RETURN),
                "esc" => press(VK_ESCAPE),
                "down" => press(VK_DOWN),
                "up" => press(VK_UP),
                "tab" => press(VK_TAB),
                other => eprintln!("不認得的按鍵 {other:?}"),
            },
            "wait" => {
                let ms: u64 = arg.parse().unwrap_or(500);
                std::thread::sleep(Duration::from_millis(ms));
            }
            "alt" => {
                if let Some(c) = arg.chars().next() {
                    send_with_modifiers(c, &[VK_MENU]);
                }
            }
            "ctrlalt" => {
                if let Some(c) = arg.chars().next() {
                    send_ctrl_alt(c);
                }
            }
            "ctrl" => {
                if let Some(c) = arg.chars().next() {
                    if let Err(e) = input::send_ctrl(c) {
                        eprintln!("送 Ctrl+{c} 失敗：{e}");
                    }
                }
            }
            "retarget" => match focus::find_window(arg) {
                Some(next) => {
                    println!("retarget -> {:?}", next.title());
                    token = next;
                }
                None => {
                    eprintln!("retarget 找不到 {arg:?}");
                    std::process::exit(1);
                }
            },
            "focus" => {
                let _ = focus::restore_focus(&token, Duration::from_millis(1500));
            }
            other => eprintln!("不認得的動作 {other:?}"),
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    println!("done");
}
