//! 按鍵注入。

use crate::focus::{is_foreground, FocusToken};
use crate::WinError;
use std::time::Duration;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, MapVirtualKeyW, SendInput, VkKeyScanExW, INPUT, INPUT_0, INPUT_KEYBOARD,
    KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC,
    VIRTUAL_KEY, VK_CONTROL, VK_LEFT, VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::GetMessageExtraInfo;

/// 送鍵失敗時的重試次數。
const SEND_RETRIES: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_millis(50);

/// 每送出這麼多字元就重新確認一次前景視窗。以 2 ms 的字元間隔計，偵測延遲約 64 ms。
const RECHECK_EVERY: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMethod {
    /// `KEYEVENTF_UNICODE`，送 VK_PACKET。適用絕大多數程式。
    Unicode,
    /// `VkKeyScanExW` + `KEYEVENTF_SCANCODE`，模擬實體按鍵。
    /// 適用只吃掃描碼的程式（部分遊戲、某些遠端桌面用戶端）。
    Virtual,
}

fn extra_info() -> usize {
    unsafe { GetMessageExtraInfo().0 as usize }
}

fn key_event(vk: VIRTUAL_KEY, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS(flags),
                time: 0,
                dwExtraInfo: extra_info(),
            },
        },
    }
}

/// 送出一組事件，失敗時重試。
fn send(events: &[INPUT]) -> Result<(), WinError> {
    let size = std::mem::size_of::<INPUT>() as i32;
    for attempt in 0..SEND_RETRIES {
        let sent = unsafe { SendInput(events, size) };
        if sent as usize == events.len() {
            return Ok(());
        }
        if attempt + 1 < SEND_RETRIES {
            std::thread::sleep(RETRY_DELAY);
        }
    }
    Err(WinError::Other(format!(
        "SendInput 只送出部分事件（共 {} 個）",
        events.len()
    )))
}

/// 送出 Ctrl+<字母>。字母以 ASCII 大寫的虛擬鍵碼送出。
pub fn send_ctrl(letter: char) -> Result<(), WinError> {
    let upper = letter.to_ascii_uppercase();
    if !upper.is_ascii_alphabetic() {
        return Err(WinError::Other(format!("不是字母：{letter:?}")));
    }
    let vk = VIRTUAL_KEY(upper as u16);
    send(&[
        key_event(VK_CONTROL, 0, 0),
        key_event(vk, 0, 0),
        key_event(vk, 0, KEYEVENTF_KEYUP.0),
        key_event(VK_CONTROL, 0, KEYEVENTF_KEYUP.0),
    ])
}

/// 送出 Ctrl+V。
pub fn send_paste() -> Result<(), WinError> {
    send_ctrl('v')
}

/// 送出 n 次左方向鍵。
pub fn send_arrow_left(n: u32) -> Result<(), WinError> {
    for _ in 0..n {
        send(&[
            key_event(VK_LEFT, 0, 0),
            key_event(VK_LEFT, 0, KEYEVENTF_KEYUP.0),
        ])?;
    }
    Ok(())
}

/// 逐字送出文字，不碰剪貼簿。
///
/// 每 [`RECHECK_EVERY`] 個字元重新確認前景視窗，不符立即中止並回
/// [`WinError::TargetChanged`]；已送出的部分無法收回。
pub fn send_text(
    text: &str,
    method: InputMethod,
    target: &FocusToken,
    delay: Duration,
) -> Result<(), WinError> {
    // 以 UTF-16 code unit 為單位：非 BMP 字元自然拆成代理對的兩次送出，接收端重組
    let units: Vec<u16> = text.encode_utf16().collect();

    for (i, unit) in units.iter().enumerate() {
        if i % RECHECK_EVERY == 0 && !is_foreground(target) {
            return Err(WinError::TargetChanged { sent: i });
        }
        match method {
            InputMethod::Unicode => send_unit_unicode(*unit)?,
            InputMethod::Virtual => send_unit_virtual(*unit)?,
        }
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
    }
    Ok(())
}

fn send_unit_unicode(unit: u16) -> Result<(), WinError> {
    send(&[
        key_event(VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE.0),
        key_event(
            VIRTUAL_KEY(0),
            unit,
            KEYEVENTF_UNICODE.0 | KEYEVENTF_KEYUP.0,
        ),
    ])
}

/// 模擬實體按鍵。字元在目前鍵盤配置打不出來時退回 Unicode 路徑。
fn send_unit_virtual(unit: u16) -> Result<(), WinError> {
    let layout = unsafe { GetKeyboardLayout(0) };
    let scan_result = unsafe { VkKeyScanExW(unit, layout) };
    if scan_result == -1 {
        return send_unit_unicode(unit);
    }

    let vk = VIRTUAL_KEY((scan_result & 0xFF) as u16);
    let modifiers = (scan_result >> 8) & 0xFF;
    let scan = unsafe { MapVirtualKeyW(vk.0 as u32, MAPVK_VK_TO_VSC) } as u16;

    let mut events: Vec<INPUT> = Vec::with_capacity(8);
    let mut held: Vec<VIRTUAL_KEY> = Vec::new();
    for (bit, key) in [(0x1, VK_SHIFT), (0x2, VK_CONTROL), (0x4, VK_MENU)] {
        if modifiers & bit != 0 {
            events.push(key_event(key, 0, 0));
            held.push(key);
        }
    }

    events.push(key_event(VIRTUAL_KEY(0), scan, KEYEVENTF_SCANCODE.0));
    events.push(key_event(
        VIRTUAL_KEY(0),
        scan,
        KEYEVENTF_SCANCODE.0 | KEYEVENTF_KEYUP.0,
    ));

    for key in held.into_iter().rev() {
        events.push(key_event(key, 0, KEYEVENTF_KEYUP.0));
    }

    send(&events)
}
