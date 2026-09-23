//! 自己建一個編輯框視窗當注入標靶。

#![cfg(windows)]
#![allow(dead_code)]

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::*;

static EDIT_HWND: AtomicIsize = AtomicIsize::new(0);

pub struct ScratchWindow {
    hwnd: isize,
    edit: isize,
}

impl ScratchWindow {
    /// 在專用執行緒上建視窗並跑訊息迴圈，回傳可供注入的控制代碼。
    ///
    /// 訊息迴圈必須跟注入分開：注入時主執行緒是忙的，
    /// 視窗要能同時處理送進來的按鍵事件。
    pub fn spawn(title: &str) -> ScratchWindow {
        let (tx, rx) = mpsc::channel::<(isize, isize)>();
        let title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();

        std::thread::spawn(move || unsafe {
            let instance = GetModuleHandleW(None).expect("module handle");
            let class = w!("InkKeepScratchWindow");

            let wc = WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                hInstance: HINSTANCE(instance.0),
                lpszClassName: class,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                ..Default::default()
            };
            RegisterClassW(&wc);

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class,
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                100,
                100,
                700,
                400,
                None,
                None,
                HINSTANCE(instance.0),
                None,
            )
            .expect("建立視窗");

            let edit = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("EDIT"),
                PCWSTR::null(),
                WS_CHILD
                    | WS_VISIBLE
                    | WINDOW_STYLE(ES_MULTILINE as u32)
                    | WINDOW_STYLE(ES_AUTOVSCROLL as u32)
                    | WINDOW_STYLE(ES_WANTRETURN as u32),
                0,
                0,
                700,
                360,
                hwnd,
                None,
                HINSTANCE(instance.0),
                None,
            )
            .expect("建立編輯框");

            EDIT_HWND.store(edit.0 as isize, Ordering::SeqCst);
            let _ = SetFocus(edit);
            tx.send((hwnd.0 as isize, edit.0 as isize))
                .expect("回報控制代碼");

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });

        let (hwnd, edit) = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("等不到視窗建立");
        ScratchWindow { hwnd, edit }
    }

    pub fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut std::ffi::c_void)
    }

    /// 直接讀編輯框的內容，不經過剪貼簿。
    pub fn text(&self) -> String {
        unsafe {
            let edit = HWND(self.edit as *mut std::ffi::c_void);
            let len = SendMessageW(edit, WM_GETTEXTLENGTH, WPARAM(0), LPARAM(0)).0 as usize;
            if len == 0 {
                return String::new();
            }
            let mut buf = vec![0u16; len + 1];
            let got = SendMessageW(
                edit,
                WM_GETTEXT,
                WPARAM(buf.len()),
                LPARAM(buf.as_mut_ptr() as isize),
            )
            .0 as usize;
            String::from_utf16_lossy(&buf[..got])
        }
    }

    pub fn clear(&self) {
        unsafe {
            let edit = HWND(self.edit as *mut std::ffi::c_void);
            let empty = w!("");
            SendMessageW(edit, WM_SETTEXT, WPARAM(0), LPARAM(empty.as_ptr() as isize));
        }
    }

    pub fn close(&self) {
        unsafe {
            let _ = PostMessageW(self.hwnd(), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        // 視窗被啟用時把焦點轉給編輯框，否則按鍵會落在父視窗上
        WM_SETFOCUS => {
            let edit = EDIT_HWND.load(Ordering::SeqCst);
            if edit != 0 {
                let _ = SetFocus(HWND(edit as *mut std::ffi::c_void));
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[allow(dead_code)]
fn main() {
    let w = ScratchWindow::spawn("TARGET-WINDOW");
    println!("視窗已開，每 2 秒印一次內容。Ctrl+C 或關掉視窗即結束。");
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let text = w.text();
        if !text.is_empty() {
            println!("內容：{text:?}");
        }
    }
}
