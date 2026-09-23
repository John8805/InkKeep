//! 視窗外框的微調。

use crate::WinError;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SendMessageW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, ICON_BIG,
    ICON_SMALL, STYLESTRUCT, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    WM_SETICON, WM_STYLECHANGING, WS_EX_DLGMODALFRAME,
};

/// 子類別化用的識別碼，隨便挑一個不會跟別人撞的數字。
const KEEP_FRAME_SUBCLASS: usize = 0x494B_4B50; // "IKKP"

/// 每次視窗的延伸樣式要被改之前，都把 `WS_EX_DLGMODALFRAME` 補回去。
///
/// 只設一次不夠：Tauri 底下的 tao 自己記著一份視窗樣式，視窗每次顯示、
/// 最大化、還原都會把整份重新套上去，我們另外加的旗標就被洗掉，
/// 標題列又冒出 Windows 內建的通用程式圖示。
unsafe extern "system" fn keep_dialog_frame(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    // WM_STYLECHANGING 的 wParam 是 GWL_STYLE 或 GWL_EXSTYLE，lParam 指向新舊樣式
    if msg == WM_STYLECHANGING && wparam.0 as isize == GWL_EXSTYLE.0 as isize {
        let styles = &mut *(lparam.0 as *mut STYLESTRUCT);
        styles.styleNew |= WS_EX_DLGMODALFRAME.0;
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

/// 拿掉標題列左上角的程式圖示。
///
/// Windows 沒有「這個視窗不要圖示」的旗標，官方做法是把視窗標成對話框外框
/// （`WS_EX_DLGMODALFRAME`）——對話框本來就不顯示圖示——再把大小圖示都設成
/// null，最後用 `SWP_FRAMECHANGED` 逼系統重畫外框。
///
/// 少了 `SWP_FRAMECHANGED` 那一步，樣式改了但畫面不會更新，要等到下一次
/// 視窗大小變動才生效。
///
/// 另外掛一個子類別程序（[`keep_dialog_frame`]），因為這個旗標會被 tao 洗掉。
pub fn hide_title_bar_icon(hwnd: isize) -> Result<(), WinError> {
    let hwnd = HWND(hwnd as *mut std::ffi::c_void);
    // SAFETY: 呼叫時 hwnd 指向的視窗仍然存在
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_DLGMODALFRAME.0 as isize);

        SendMessageW(hwnd, WM_SETICON, WPARAM(ICON_SMALL as usize), LPARAM(0));
        SendMessageW(hwnd, WM_SETICON, WPARAM(ICON_BIG as usize), LPARAM(0));

        // 必須在擁有這個視窗的執行緒上呼叫
        if !SetWindowSubclass(hwnd, Some(keep_dialog_frame), KEEP_FRAME_SUBCLASS, 0).as_bool() {
            return Err(WinError::Other("無法攔截視窗樣式變更".into()));
        }

        SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
        )
        .map_err(|e| WinError::Other(format!("重畫視窗外框失敗：{e}")))
    }
}
