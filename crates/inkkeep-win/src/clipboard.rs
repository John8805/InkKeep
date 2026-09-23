//! 剪貼簿讀寫。
//!
//! 三個排除標記必須與文字在同一次 `OpenClipboard` 工作階段內設定
//! （`EmptyClipboard` 之後、`CloseClipboard` 之前）。

use crate::WinError;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

/// `CF_UNICODETEXT`。
const CF_UNICODETEXT: u32 = 13;

/// 擋掉 Windows 內建剪貼簿歷史（Win+V）與雲端剪貼簿的三個格式。
/// 擋不掉第三方剪貼簿管理員。
const EXCLUDE_MONITOR: &str = "ExcludeClipboardContentFromMonitorProcessing";
const NO_HISTORY: &str = "CanIncludeInClipboardHistory";
const NO_CLOUD: &str = "CanUploadToCloudClipboard";

/// 我們動手之前剪貼簿裡的文字。
///
/// 不含序號：判斷「有沒有被別人改過」要比對的是**我們寫入之後**的序號，
/// 那個值由 [`set_text`] 回傳，呼叫端再交給 [`restore`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipSnapshot {
    pub(crate) text: Option<String>,
}

impl ClipSnapshot {
    /// 內容摘要：前 40 個字元，控制字元換成空白。
    pub fn text_preview(&self) -> Option<String> {
        self.text.as_ref().map(|t| {
            let flat: String = t
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            flat.chars().take(40).collect()
        })
    }

    pub fn had_text(&self) -> bool {
        self.text.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreOutcome {
    Restored,
    /// 剪貼簿在這段期間被別人改過，放棄還原以免覆蓋使用者的新內容。
    Skipped,
}

pub fn sequence_number() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

/// 開啟剪貼簿的 RAII 包裝。Windows 的剪貼簿是全域鎖，忘記關會卡住整個系統的複製貼上。
struct ClipboardGuard;

impl ClipboardGuard {
    fn open() -> Result<Self, WinError> {
        unsafe {
            // 別的程式可能正握著剪貼簿，重試幾次
            for attempt in 0..10 {
                if OpenClipboard(None).is_ok() {
                    return Ok(ClipboardGuard);
                }
                std::thread::sleep(std::time::Duration::from_millis(10 * (attempt + 1)));
            }
            Err(WinError::ClipboardBusy)
        }
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

pub fn snapshot() -> Result<ClipSnapshot, WinError> {
    Ok(ClipSnapshot { text: read_text()? })
}

pub fn read_text() -> Result<Option<String>, WinError> {
    let _guard = ClipboardGuard::open()?;
    unsafe {
        let Ok(handle) = GetClipboardData(CF_UNICODETEXT) else {
            // 剪貼簿是空的或裝著非文字內容
            return Ok(None);
        };
        if handle.0.is_null() {
            return Ok(None);
        }
        let hglobal = HGLOBAL(handle.0);
        let ptr = GlobalLock(hglobal) as *const u16;
        if ptr.is_null() {
            return Ok(None);
        }
        let mut len = 0usize;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
        let _ = GlobalUnlock(hglobal);
        Ok(Some(text))
    }
}

/// 寫入文字，同時設定三個排除標記。回傳寫入後的序號。
pub fn set_text(text: &str) -> Result<u32, WinError> {
    let _guard = ClipboardGuard::open()?;
    unsafe {
        EmptyClipboard().map_err(|e| WinError::Other(e.to_string()))?;

        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = std::mem::size_of_val(wide.as_slice());
        let hglobal =
            GlobalAlloc(GMEM_MOVEABLE, bytes).map_err(|e| WinError::Other(e.to_string()))?;
        let ptr = GlobalLock(hglobal) as *mut u16;
        if ptr.is_null() {
            return Err(WinError::Other("GlobalLock 失敗".into()));
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
        let _ = GlobalUnlock(hglobal);

        // 交出所有權：成功之後不可以自己 GlobalFree
        SetClipboardData(CF_UNICODETEXT, HANDLE(hglobal.0))
            .map_err(|e| WinError::Other(e.to_string()))?;

        set_marker(EXCLUDE_MONITOR, &[1u8])?;
        set_marker(NO_HISTORY, &0u32.to_ne_bytes())?;
        set_marker(NO_CLOUD, &0u32.to_ne_bytes())?;
    }
    // 關掉剪貼簿之後序號才會定案
    drop(_guard);
    Ok(sequence_number())
}

/// 還原剪貼簿。`our_seq` 是 [`set_text`] 回傳的序號；
/// 目前序號與它不同表示使用者自己複製過東西，這時放棄還原，不覆蓋他的新內容。
pub fn restore(snap: &ClipSnapshot, our_seq: u32) -> Result<RestoreOutcome, WinError> {
    if sequence_number() != our_seq {
        return Ok(RestoreOutcome::Skipped);
    }
    match &snap.text {
        Some(text) => {
            set_text(text)?;
            Ok(RestoreOutcome::Restored)
        }
        // 原本不是文字內容（或空的）：清掉我們放進去的東西，不假裝能還原圖片
        None => {
            clear()?;
            Ok(RestoreOutcome::Restored)
        }
    }
}

pub fn clear() -> Result<(), WinError> {
    let _guard = ClipboardGuard::open()?;
    unsafe { EmptyClipboard().map_err(|e| WinError::Other(e.to_string())) }
}

/// 在已開啟的剪貼簿工作階段內放一個自訂格式。
unsafe fn set_marker(name: &str, payload: &[u8]) -> Result<(), WinError> {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let format = RegisterClipboardFormatW(PCWSTR(wide.as_ptr()));
    if format == 0 {
        // 註冊不起來就算了，排除標記是加強項而非必要功能
        return Ok(());
    }
    let hglobal =
        GlobalAlloc(GMEM_MOVEABLE, payload.len()).map_err(|e| WinError::Other(e.to_string()))?;
    let ptr = GlobalLock(hglobal) as *mut u8;
    if ptr.is_null() {
        return Err(WinError::Other("GlobalLock 失敗".into()));
    }
    std::ptr::copy_nonoverlapping(payload.as_ptr(), ptr, payload.len());
    let _ = GlobalUnlock(hglobal);
    SetClipboardData(format, HANDLE(hglobal.0)).map_err(|e| WinError::Other(e.to_string()))?;
    Ok(())
}
