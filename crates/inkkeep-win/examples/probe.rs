//! 手動驗證 inkkeep-win 的工具。
//!
//! 用法（從另一個視窗執行，因為要測的就是「送進別的視窗」）：
//!
//! ```text
//! cargo run -p inkkeep-win --example probe -- focus
//! cargo run -p inkkeep-win --example probe -- clipboard
//! cargo run -p inkkeep-win --example probe -- paste "要貼的文字"
//! cargo run -p inkkeep-win --example probe -- type "要打的文字"
//! cargo run -p inkkeep-win --example probe -- type-virtual "要打的文字"
//! ```
//!
//! `paste` 與 `type` 會倒數 3 秒，期間請切到目標視窗（記事本之類）。
//!
//! `verify-*` 是自動化的：自己建一個編輯框視窗當標靶，測完關掉。

#[path = "scratch_window.rs"]
mod scratch_window;

use scratch_window::ScratchWindow;
use inkkeep_win::{clipboard, focus, input, InputMethod};
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first().map(String::as_str) else {
        eprintln!("用法：probe <focus|clipboard|paste|type|type-virtual|verify-paste|verify-type|verify-type-virtual> [文字]");
        std::process::exit(2);
    };
    let text = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "inkkeep 測試".into());

    match cmd {
        "focus" => probe_focus(),
        "clipboard" => probe_clipboard(),
        "paste" => probe_paste(&text),
        "type" => probe_type(&text, InputMethod::Unicode),
        "type-virtual" => probe_type(&text, InputMethod::Virtual),
        "verify-paste" => probe_verify(&text, None),
        "verify-type" => probe_verify(&text, Some(InputMethod::Unicode)),
        "verify-type-virtual" => probe_verify(&text, Some(InputMethod::Virtual)),
        "verify-interrupt" => probe_interrupt(),
        other => {
            eprintln!("不認得的指令 {other:?}");
            std::process::exit(2);
        }
    }
}

fn countdown(secs: u32) {
    for n in (1..=secs).rev() {
        println!("{n}… 請切到目標視窗");
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn probe_focus() {
    countdown(3);
    match focus::capture_focus() {
        Ok(token) => {
            println!("前景視窗 pid={} 標題={:?}", token.pid(), token.title());
            match focus::check_target(&token) {
                Ok(()) => println!("完整性等級檢查：可以送入"),
                Err(e) => println!("完整性等級檢查：{e}"),
            }
            println!("目前是否仍為前景：{}", focus::is_foreground(&token));
        }
        Err(e) => println!("擷取失敗：{e}"),
    }
}

fn probe_clipboard() {
    let before = clipboard::snapshot().expect("讀剪貼簿");
    println!("原本內容：{:?}", before.text_preview());

    let our_seq = clipboard::set_text("inkkeep 寫進來的內容").expect("寫剪貼簿");
    println!("寫入後讀回：{:?}", clipboard::read_text().expect("讀回"));
    println!("現在去按 Win+V，這一筆不該出現在歷史裡。按 Enter 繼續還原…");
    let _ = std::io::stdin().read_line(&mut String::new());

    match clipboard::restore(&before, our_seq) {
        Ok(outcome) => println!("還原結果：{outcome:?}"),
        Err(e) => println!("還原失敗：{e}"),
    }
}

fn probe_paste(text: &str) {
    countdown(3);
    let token = match focus::capture_focus() {
        Ok(t) => t,
        Err(e) => {
            println!("擷取前景失敗：{e}");
            return;
        }
    };
    println!("目標：{:?}", token.title());

    if let Err(e) = focus::check_target(&token) {
        println!("不能送進這個視窗：{e}");
        return;
    }

    let snap = clipboard::snapshot().expect("快照");
    let our_seq = clipboard::set_text(text).expect("寫剪貼簿");

    if let Err(e) = focus::restore_focus(&token, Duration::from_millis(300)) {
        println!("還原焦點失敗：{e}（內容留在剪貼簿）");
        return;
    }
    if let Err(e) = input::send_paste() {
        println!("送出 Ctrl+V 失敗：{e}");
        return;
    }
    println!("已貼上");

    std::thread::sleep(Duration::from_millis(150));
    match clipboard::restore(&snap, our_seq) {
        Ok(outcome) => println!("剪貼簿還原：{outcome:?}"),
        Err(e) => println!("剪貼簿還原失敗：{e}"),
    }
}

fn probe_type(text: &str, method: InputMethod) {
    countdown(3);
    let token = match focus::capture_focus() {
        Ok(t) => t,
        Err(e) => {
            println!("擷取前景失敗：{e}");
            return;
        }
    };
    println!("目標：{:?}，方式：{method:?}", token.title());

    if let Err(e) = focus::check_target(&token) {
        println!("不能送進這個視窗：{e}");
        return;
    }
    if let Err(e) = focus::restore_focus(&token, Duration::from_millis(300)) {
        println!("還原焦點失敗：{e}");
        return;
    }

    let before = clipboard::sequence_number();
    match input::send_text(text, method, &token, Duration::from_millis(2)) {
        Ok(()) => println!(
            "已輸入，剪貼簿序號未變：{}",
            clipboard::sequence_number() == before
        ),
        Err(e) => println!("輸入中止：{e}"),
    }
}

/// 端對端驗證：在自己建的編輯框視窗上送入內容，再用 `WM_GETTEXT` 讀回來比對。
fn probe_verify(text: &str, method: Option<InputMethod>) {
    let window = ScratchWindow::spawn("TARGET-WINDOW");
    // 等視窗真的可以被指定為前景
    std::thread::sleep(Duration::from_millis(400));

    let Some(token) = focus::find_window("TARGET-WINDOW") else {
        println!("找不到自己建的視窗");
        window.close();
        std::process::exit(1);
    };
    println!("目標：{:?} pid={}", token.title(), token.pid());

    if let Err(e) = focus::restore_focus(&token, Duration::from_millis(1000)) {
        println!("切換焦點失敗：{e}");
        window.close();
        std::process::exit(1);
    }
    window.clear();

    match method {
        Some(m) => {
            println!("方式：直接輸入 {m:?}");
            let before_seq = clipboard::sequence_number();
            if let Err(e) = input::send_text(text, m, &token, Duration::from_millis(2)) {
                println!("輸入中止：{e}");
                window.close();
                std::process::exit(1);
            }
            if clipboard::sequence_number() != before_seq {
                println!("❌ 直接輸入竟然動到了剪貼簿");
                window.close();
                std::process::exit(1);
            }
        }
        None => {
            println!("方式：剪貼簿貼上");
            let snap = clipboard::snapshot().expect("快照");
            let our_seq = clipboard::set_text(text).expect("寫剪貼簿");
            input::send_paste().expect("送 Ctrl+V");
            std::thread::sleep(Duration::from_millis(150));
            match clipboard::restore(&snap, our_seq) {
                Ok(o) => println!("剪貼簿還原：{o:?}"),
                Err(e) => println!("剪貼簿還原失敗：{e}"),
            }
        }
    }

    std::thread::sleep(Duration::from_millis(250));
    let got = window.text();
    let ok = got == text;
    if ok {
        println!("✅ 內容相符（{} 個字元）", got.chars().count());
    } else {
        println!(
            "❌ 不相符
  預期：{text:?}
  實得：{got:?}"
        );
    }

    window.close();
    if !ok {
        std::process::exit(1);
    }
}

/// 驗證送出途中切走視窗會被攔下來。
fn probe_interrupt() {
    let window = ScratchWindow::spawn("TARGET-WINDOW");
    std::thread::sleep(Duration::from_millis(400));
    let Some(token) = focus::find_window("TARGET-WINDOW") else {
        println!("找不到自己建的視窗");
        return;
    };
    focus::restore_focus(&token, Duration::from_millis(1000)).expect("切換焦點");
    window.clear();

    // 另一個執行緒在中途把視窗關掉，模擬使用者切走
    let hwnd_gone = std::thread::spawn({
        let w = window.hwnd().0 as isize;
        move || {
            std::thread::sleep(Duration::from_millis(200));
            unsafe {
                use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
                let _ = PostMessageW(
                    windows::Win32::Foundation::HWND(w as *mut std::ffi::c_void),
                    WM_CLOSE,
                    windows::Win32::Foundation::WPARAM(0),
                    windows::Win32::Foundation::LPARAM(0),
                );
            }
        }
    });

    let long: String = "0123456789".repeat(40);
    match input::send_text(
        &long,
        InputMethod::Unicode,
        &token,
        Duration::from_millis(5),
    ) {
        Ok(()) => println!("❌ 應該被中斷卻送完了"),
        Err(e) => println!("✅ 如預期中止：{e}"),
    }
    let _ = hwnd_gone.join();
}
