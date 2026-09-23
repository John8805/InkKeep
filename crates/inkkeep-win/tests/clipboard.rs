//! 剪貼簿的自動化測試。
//!
//! 剪貼簿是全系統共用的狀態，這些步驟不能平行跑，所以全部塞在一個測試函式裡循序執行。
//! 跑完會把剪貼簿還原成測試前的樣子。

#![cfg(windows)]

use inkkeep_win::clipboard::{self, RestoreOutcome};

#[test]
fn clipboard_roundtrip_and_restore_semantics() {
    // 先把使用者原本的內容記下來，測試結束再放回去
    let user_had = clipboard::snapshot().expect("讀剪貼簿");

    write_then_read_back();
    restore_puts_the_old_text_back();
    restore_skips_when_someone_else_wrote();
    restore_clears_when_there_was_no_text();
    long_and_multiline_text_survives();

    if let Some(text) = user_had.text_preview() {
        // 只是盡力，測試已經動過剪貼簿了
        let _ = clipboard::set_text(&text);
    }
}

fn write_then_read_back() {
    clipboard::set_text("第一段文字").expect("寫入");
    assert_eq!(
        clipboard::read_text().expect("讀回"),
        Some("第一段文字".to_string())
    );
}

fn restore_puts_the_old_text_back() {
    clipboard::set_text("原本的內容").expect("先放原本的");
    let snap = clipboard::snapshot().expect("快照");
    assert!(snap.had_text());

    let our_seq = clipboard::set_text("我們塞進去的").expect("寫入");
    assert_eq!(
        clipboard::read_text().expect("讀回"),
        Some("我們塞進去的".to_string())
    );

    assert_eq!(
        clipboard::restore(&snap, our_seq).expect("還原"),
        RestoreOutcome::Restored
    );
    assert_eq!(
        clipboard::read_text().expect("讀回"),
        Some("原本的內容".to_string())
    );
}

/// 使用者在我們還原之前自己複製了東西，這時不該覆蓋他的新內容。
fn restore_skips_when_someone_else_wrote() {
    clipboard::set_text("原本的內容").expect("先放原本的");
    let snap = clipboard::snapshot().expect("快照");
    let our_seq = clipboard::set_text("我們塞進去的").expect("寫入");

    // 模擬使用者按 Ctrl+C
    clipboard::set_text("使用者後來複製的").expect("第三方寫入");

    assert_eq!(
        clipboard::restore(&snap, our_seq).expect("還原"),
        RestoreOutcome::Skipped
    );
    assert_eq!(
        clipboard::read_text().expect("讀回"),
        Some("使用者後來複製的".to_string()),
        "使用者的新內容被蓋掉了"
    );
}

/// 原本是空的或非文字內容時，還原等於清空——不假裝能把圖片放回去。
fn restore_clears_when_there_was_no_text() {
    clipboard::clear().expect("清空");
    let snap = clipboard::snapshot().expect("快照");
    assert!(!snap.had_text());

    let our_seq = clipboard::set_text("暫時的內容").expect("寫入");
    assert_eq!(
        clipboard::restore(&snap, our_seq).expect("還原"),
        RestoreOutcome::Restored
    );
    assert_eq!(clipboard::read_text().expect("讀回"), None);
}

fn long_and_multiline_text_survives() {
    let text = format!(
        "第一行\r\n第二行\n\t縮排\n{}\nemoji 🙂 與非 BMP 字元",
        "長".repeat(5000)
    );
    clipboard::set_text(&text).expect("寫入");
    assert_eq!(clipboard::read_text().expect("讀回"), Some(text));
}
