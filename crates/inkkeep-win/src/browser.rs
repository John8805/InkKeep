//! 用預設瀏覽器開啟網址。

use crate::WinError;
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// 只接受 http、https 開頭的網址。其他協定（`file:`、自訂協定）交給 Windows 開，
/// 可能會啟動本機程式。
pub fn is_web_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    ["http://", "https://"]
        .iter()
        .any(|scheme| lower.len() > scheme.len() && lower.starts_with(scheme))
}

/// 用預設瀏覽器開啟。瀏覽器已經開著時通常開成新分頁。
pub fn open_url(url: &str) -> Result<(), WinError> {
    if !is_web_url(url) {
        return Err(WinError::Other("unsupported-url".into()));
    }
    // SAFETY: 參數都是呼叫期間有效的 UTF-16 字串或 null
    let result = unsafe {
        ShellExecuteW(
            HWND::default(),
            w!("open"),
            &HSTRING::from(url.trim()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // 回傳值大於 32 表示成功，其餘是錯誤代碼
    let code = result.0 as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(WinError::Other(format!("ShellExecute {code}")))
    }
}

#[cfg(test)]
mod tests {
    use super::is_web_url;

    #[test]
    fn only_web_urls_are_accepted() {
        for ok in ["https://example.com", "HTTP://example.com/a?b=c", "  https://x.y  "] {
            assert!(is_web_url(ok), "{ok}");
        }
        for bad in [
            "file:///C:/Windows/System32/calc.exe",
            "ms-settings:",
            "javascript:alert(1)",
            "C:\\Windows\\notepad.exe",
            "https://",
            "",
        ] {
            assert!(!is_web_url(bad), "{bad}");
        }
    }
}
