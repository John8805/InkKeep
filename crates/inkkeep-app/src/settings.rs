//! settings.toml 的讀寫。
//!
//! 壞掉的欄位回退預設值，設定檔手改壞了程式照樣能啟動。

use serde::{Deserialize, Serialize};
use inkkeep_core::gen::GenOpts;
use std::collections::BTreeMap;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub vault_path: Option<String>,
    pub hotkey: String,
    pub theme: String,
    /// "auto" 或 `i18n::SUPPORTED` 裡的語言代碼
    pub language: String,
    pub launch_at_login: bool,
    pub lock_after_minutes: u32,
    /// true 就不把第一層金鑰寫進認證管理員，每次啟動都問主密碼。
    pub require_password_at_startup: bool,
    pub window_position: Option<(i32, i32)>,
    pub window_size: Option<(u32, u32)>,
    pub password_gen: GenOpts,
    pub inject: InjectSettings,
    /// 搜尋視窗的快捷鍵：動作代碼 → 組合鍵（例如 `"copy" = "Shift+Enter"`）。
    /// 只記使用者改過的；空字串表示這個動作不指定。預設值在前端 `shortcuts.svelte.js`。
    pub shortcuts: BTreeMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            version: 1,
            vault_path: None,
            hotkey: "Alt+Period".into(),
            theme: "system".into(),
            language: "auto".into(),
            launch_at_login: false,
            lock_after_minutes: 15,
            require_password_at_startup: false,
            window_position: None,
            window_size: None,
            password_gen: GenOpts::default(),
            inject: InjectSettings::default(),
            shortcuts: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct InjectSettings {
    pub focus_timeout_ms: u64,
    pub paste_settle_ms: u64,
    pub type_delay_ms: u64,
    /// 貼上後等多久才還原原本的剪貼簿文字。太短的話，目標程式會貼到還原後的內容。
    pub restore_delay_ms: u64,
    /// true 表示敏感項目預設走 Virtual 模式。
    /// 中文輸入法下 Virtual 無效，所以預設是 false（Unicode）。
    pub virtual_input: bool,
}

impl Default for InjectSettings {
    fn default() -> Self {
        InjectSettings {
            focus_timeout_ms: 300,
            paste_settle_ms: 30,
            type_delay_ms: 2,
            restore_delay_ms: 300,
            virtual_input: false,
        }
    }
}

impl InjectSettings {
    pub fn load(app: &AppHandle) -> Self {
        Settings::load(app).inject
    }
}

impl Settings {
    pub fn path(app: &AppHandle) -> PathBuf {
        app.path()
            .app_config_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("settings.toml")
    }

    /// 解析失敗時回退預設值並重寫檔案；值域不合法的欄位由 `sanitized` 修正。
    pub fn load(app: &AppHandle) -> Self {
        let path = Self::path(app);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Settings::default();
        };
        match toml::from_str::<Settings>(&text) {
            Ok(s) => s.sanitized(),
            Err(e) => {
                eprintln!("settings.toml 解析失敗，改用預設值：{e}");
                let fallback = Settings::default();
                let _ = fallback.save(app);
                fallback
            }
        }
    }

    pub fn save(&self, app: &AppHandle) -> std::io::Result<()> {
        let path = Self::path(app);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self).expect("設定可序列化");
        std::fs::write(path, text)
    }

    /// 把值域不合法的欄位夾回範圍內或回退預設值。
    pub fn sanitized(mut self) -> Self {
        let d = InjectSettings::default();
        self.inject.focus_timeout_ms = self.inject.focus_timeout_ms.clamp(0, 5000);
        self.inject.paste_settle_ms = self.inject.paste_settle_ms.clamp(0, 2000);
        self.inject.type_delay_ms = self.inject.type_delay_ms.clamp(0, 100);
        self.inject.restore_delay_ms = self.inject.restore_delay_ms.clamp(50, 5000);
        if self.inject.focus_timeout_ms == 0 {
            self.inject.focus_timeout_ms = d.focus_timeout_ms;
        }
        self.lock_after_minutes = self.lock_after_minutes.min(24 * 60);
        if self.hotkey.trim().is_empty() {
            self.hotkey = Settings::default().hotkey;
        }
        if !matches!(self.theme.as_str(), "system" | "light" | "dark") {
            self.theme = "system".into();
        }
        if self.language != "auto" && !crate::i18n::SUPPORTED.contains(&self.language.as_str()) {
            self.language = "auto".into();
        }
        self
    }

    pub fn resolved_vault_path(&self, app: &AppHandle) -> PathBuf {
        match &self.vault_path {
            Some(p) if !p.trim().is_empty() => PathBuf::from(p),
            _ => app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("vault.kdbx"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_range_values_are_clamped() {
        let s = Settings {
            hotkey: "  ".into(),
            theme: "purple".into(),
            language: "klingon".into(),
            lock_after_minutes: 999_999,
            inject: InjectSettings {
                focus_timeout_ms: 0,
                paste_settle_ms: 99_999,
                type_delay_ms: 5_000,
                ..InjectSettings::default()
            },
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(s.hotkey, "Alt+Period", "空白快捷鍵要回預設");
        assert_eq!(s.theme, "system", "不認得的主題要回預設");
        assert_eq!(s.language, "auto", "不認得的語言要回預設");
        assert_eq!(s.lock_after_minutes, 24 * 60, "鎖定時間上限是一天");
        assert_eq!(
            s.inject.focus_timeout_ms,
            InjectSettings::default().focus_timeout_ms,
            "0 會讓輪詢立刻放棄，要回預設"
        );
        assert_eq!(s.inject.paste_settle_ms, 2000);
        assert_eq!(s.inject.type_delay_ms, 100);
    }

    #[test]
    fn legal_values_pass_through_untouched() {
        let s = Settings {
            hotkey: "Ctrl+Alt+K".into(),
            theme: "dark".into(),
            language: "en".into(),
            lock_after_minutes: 5,
            inject: InjectSettings {
                type_delay_ms: 7,
                ..InjectSettings::default()
            },
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(s.hotkey, "Ctrl+Alt+K");
        assert_eq!(s.theme, "dark");
        assert_eq!(s.language, "en");
        assert_eq!(s.lock_after_minutes, 5);
        assert_eq!(s.inject.type_delay_ms, 7);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let partial: Settings = toml::from_str(
            "version = 1
hotkey = \"Alt+Space\"",
        )
        .unwrap();
        assert_eq!(partial.hotkey, "Alt+Space");
        assert_eq!(
            partial.lock_after_minutes,
            Settings::default().lock_after_minutes
        );
        assert_eq!(partial.password_gen.length, 20);
    }

    #[test]
    fn unknown_fields_do_not_break_parsing() {
        let text = "version = 1
some_future_field = 42
";
        assert!(toml::from_str::<Settings>(text).is_ok());
    }
}
