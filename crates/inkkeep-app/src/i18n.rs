//! 後端這一側的介面字串。其餘介面字串在前端的語言包 `ui/src/locales/`。
//!
//! 新增語言時這裡跟 `ui/src/locales/` 要一起加，否則工作列會退回英文。

/// 支援的語言代碼。`settings.language` 只接受這些值或 `"auto"`。
pub const SUPPORTED: &[&str] = &["zh-Hant", "en"];

/// 把設定值解析成實際要用的語言。`"auto"` 或認不得的值交給系統語系決定。
pub fn resolve(setting: &str) -> &'static str {
    match setting {
        "zh-Hant" => "zh-Hant",
        "en" => "en",
        _ => from_system(),
    }
}

/// 問 Windows 目前使用者的 UI 語言。拿不到就用英文。
fn from_system() -> &'static str {
    match inkkeep_win::locale::system_tag() {
        Some(tag) => match_tag(&tag.to_lowercase()),
        None => "en",
    }
}

/// 語系標籤對到語言包。規則要跟前端 `i18n.svelte.js` 的 `detect()` 一致。
fn match_tag(tag: &str) -> &'static str {
    const TRADITIONAL: &[&str] = &["zh-hant", "zh-tw", "zh-hk", "zh-mo"];
    if TRADITIONAL.iter().any(|p| tag.starts_with(p)) {
        return "zh-Hant";
    }
    "en"
}

/// 「已經在背景跑了」那則通知的標題與內文。
pub fn running_in_background(lang: &str) -> (&'static str, &'static str) {
    match lang {
        "zh-Hant" => ("InkKeep", "InkKeep 正在背景執行。按快捷鍵叫出視窗。"),
        _ => ("InkKeep", "InkKeep is running in the background. Press the hotkey to open it."),
    }
}

/// 第一次開檔時自動建立的工作區名稱。
pub fn default_workspace(lang: &str) -> &'static str {
    match lang {
        "zh-Hant" => "個人",
        _ => "Personal",
    }
}

pub struct Tray {
    pub show: &'static str,
    pub lock: &'static str,
    pub quit: &'static str,
}

pub fn tray(lang: &str) -> Tray {
    match lang {
        "zh-Hant" => Tray {
            show: "顯示視窗",
            lock: "鎖定密碼",
            quit: "離開",
        },
        _ => Tray {
            show: "Show window",
            lock: "Lock passwords",
            quit: "Quit",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_codes_win_over_the_system_locale() {
        assert_eq!(resolve("zh-Hant"), "zh-Hant");
        assert_eq!(resolve("en"), "en");
    }

    #[test]
    fn unknown_codes_fall_back_to_the_system_locale() {
        let auto = resolve("auto");
        assert!(SUPPORTED.contains(&auto));
        assert_eq!(resolve("klingon"), auto);
        assert_eq!(resolve(""), auto);
    }

    /// 繁體的幾種寫法都要對到同一個語言包；簡體沒有語言包，落到英文。
    #[test]
    fn traditional_chinese_tags_all_map_to_one_bundle() {
        for tag in ["zh-tw", "zh-hant", "zh-hant-hk", "zh-hk", "zh-mo"] {
            assert_eq!(match_tag(tag), "zh-Hant", "{tag}");
        }
        assert_eq!(match_tag("zh-cn"), "en");
        assert_eq!(match_tag("ja-jp"), "en");
    }

    #[test]
    fn every_supported_language_has_a_tray_menu() {
        for lang in SUPPORTED {
            let menu = tray(lang);
            assert!(!menu.show.is_empty() && !menu.lock.is_empty() && !menu.quit.is_empty());
        }
    }

    #[test]
    fn every_supported_language_has_a_default_workspace_name() {
        for lang in SUPPORTED {
            assert!(!default_workspace(lang).trim().is_empty(), "{lang}");
        }
    }

    #[test]
    fn every_supported_language_has_the_background_notice() {
        for lang in SUPPORTED {
            let (title, body) = running_in_background(lang);
            assert!(!title.is_empty() && !body.is_empty(), "{lang}");
        }
    }
}
