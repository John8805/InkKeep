//! Windows 平台整合：前景視窗、剪貼簿、按鍵注入。

#![cfg(windows)]

pub mod browser;
pub mod chrome;
pub mod clipboard;
pub mod credstore;
pub mod focus;
pub mod input;
pub mod locale;
pub mod toast;

pub use clipboard::{ClipSnapshot, RestoreOutcome};
pub use focus::FocusToken;
pub use input::InputMethod;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WinError {
    #[error("沒有前景視窗")]
    NoForeground,
    #[error("目標視窗已不存在")]
    WindowGone,
    #[error("等不到焦點切回目標視窗")]
    RestoreTimeout,
    /// UIPI：本行程無法對完整性等級更高的視窗注入輸入。
    #[error("目標視窗以較高權限執行，無法送入內容")]
    ElevatedTarget,
    /// 送出途中使用者切換了視窗。已送出的 `sent` 個字元無法收回。
    #[error("送出途中前景視窗改變，已送出 {sent} 個字元")]
    TargetChanged { sent: usize },
    #[error("剪貼簿正被其他程式佔用")]
    ClipboardBusy,
    #[error("{0}")]
    Other(String),
}
