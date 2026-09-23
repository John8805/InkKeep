//! 依視窗標題把視窗帶到前景。
#![cfg(windows)]

use inkkeep_win::focus;
use std::time::Duration;

fn main() {
    let Some(title) = std::env::args().nth(1) else {
        eprintln!("用法：activate <視窗標題子字串>");
        std::process::exit(2);
    };
    match focus::find_window(&title) {
        Some(token) => {
            println!("找到：{:?} pid={}", token.title(), token.pid());
            match focus::restore_focus(&token, Duration::from_millis(1500)) {
                Ok(()) => println!("已帶到前景"),
                Err(e) => {
                    println!("失敗：{e}");
                    std::process::exit(1);
                }
            }
        }
        None => {
            println!("找不到標題含 {title:?} 的視窗");
            std::process::exit(1);
        }
    }
}
