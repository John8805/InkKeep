//! 目前使用者的 UI 語系。

use windows::Win32::Globalization::GetUserDefaultLocaleName;

/// BCP-47 標籤，例如 `zh-TW`、`en-US`。拿不到回 `None`。
pub fn system_tag() -> Option<String> {
    // LOCALE_NAME_MAX_LENGTH
    let mut buf = [0u16; 85];
    // SAFETY: windows crate 這個綁定直接吃切片，長度由它自己算
    let len = unsafe { GetUserDefaultLocaleName(&mut buf) };
    if len <= 1 {
        return None;
    }
    // 回傳的長度含結尾的 NUL
    Some(String::from_utf16_lossy(&buf[..(len - 1) as usize]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 測試機器的語系是什麼不知道，但格式一定是 `xx` 或 `xx-YY` 這種。
    #[test]
    fn returns_a_plausible_bcp47_tag() {
        let Some(tag) = system_tag() else {
            return; // 沒有語系設定的環境（極少見）就跳過
        };
        assert!(tag.len() >= 2, "語系標籤太短：{tag:?}");
        assert!(
            tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "語系標籤含意外字元：{tag:?}"
        );
    }
}
