//! 前景視窗的擷取、還原與完整性等級檢查。

use crate::WinError;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows::Win32::Security::{
    GetTokenInformation, TokenIntegrityLevel, SID_AND_ATTRIBUTES, TOKEN_MANDATORY_LABEL,
    TOKEN_QUERY,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::Foundation::RECT;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, BringWindowToTop, GetForegroundWindow, GetWindowRect, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindow, SetForegroundWindow, ShowWindow, SW_RESTORE,
};

/// 前景視窗的身分。
///
/// HWND 在視窗關閉後會被系統重新分配給新視窗，所以一併記 PID；
/// 只比 HWND 有機會把焦點還給一個完全不相干的視窗。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusToken {
    pub(crate) hwnd: isize,
    pub(crate) pid: u32,
    pub(crate) title: String,
}

impl FocusToken {
    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub(crate) fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut std::ffi::c_void)
    }
}

/// 目標視窗中心點的螢幕座標。視窗已經關掉就回 `None`。
pub fn center_point(token: &FocusToken) -> Option<(f64, f64)> {
    let mut rect = RECT::default();
    // SAFETY: hwnd 可能已經失效，GetWindowRect 會回 Err，不會讀到壞記憶體
    unsafe { GetWindowRect(token.hwnd(), &mut rect) }.ok()?;
    Some((
        f64::from(rect.left + rect.right) / 2.0,
        f64::from(rect.top + rect.bottom) / 2.0,
    ))
}

pub fn capture_focus() -> Result<FocusToken, WinError> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return Err(WinError::NoForeground);
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return Err(WinError::NoForeground);
        }
        Ok(FocusToken {
            hwnd: hwnd.0 as isize,
            pid,
            title: window_title(hwnd),
        })
    }
}

/// 目前前景視窗是不是 token 指的那一個。
pub fn is_foreground(token: &FocusToken) -> bool {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0 != token.hwnd().0 {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        pid == token.pid
    }
}

/// 把前景還給 token 對應的視窗，並輪詢確認真的切過去了。
pub fn restore_focus(token: &FocusToken, timeout: Duration) -> Result<(), WinError> {
    unsafe {
        let hwnd = token.hwnd();
        if !IsWindow(hwnd).as_bool() {
            return Err(WinError::WindowGone);
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid != token.pid {
            // HWND 被回收給別的行程了
            return Err(WinError::WindowGone);
        }

        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = AllowSetForegroundWindow(token.pid);
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);

        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if is_foreground(token) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Err(WinError::RestoreTimeout)
    }
}

/// 目標視窗的完整性等級高於本行程時，UIPI 會擋掉所有輸入注入。
pub fn check_target(token: &FocusToken) -> Result<(), WinError> {
    let ours = process_integrity_level(unsafe { GetCurrentProcess() })?;
    let theirs = match open_target(token.pid) {
        Ok(handle) => {
            let level = process_integrity_level(handle);
            unsafe {
                let _ = CloseHandle(handle);
            }
            level?
        }
        // 開不了 handle 本身就是權限不足的徵兆
        Err(_) => return Err(WinError::ElevatedTarget),
    };
    if theirs > ours {
        Err(WinError::ElevatedTarget)
    } else {
        Ok(())
    }
}

fn open_target(pid: u32) -> Result<HANDLE, WinError> {
    unsafe {
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
            .map_err(|e| WinError::Other(e.to_string()))
    }
}

/// 回傳 RID（SECURITY_MANDATORY_*_RID），數字越大權限越高。
fn process_integrity_level(process: HANDLE) -> Result<u32, WinError> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token)
            .map_err(|e| WinError::Other(e.to_string()))?;

        let mut needed = 0u32;
        // 先問需要多大的緩衝區；這裡必定回 false，只取 needed
        let _ = GetTokenInformation(token, TokenIntegrityLevel, None, 0, &mut needed);
        let mut buf = vec![0u8; needed as usize];
        let result = GetTokenInformation(
            token,
            TokenIntegrityLevel,
            Some(buf.as_mut_ptr() as *mut _),
            needed,
            &mut needed,
        );
        let _ = CloseHandle(token);
        result.map_err(|e| WinError::Other(e.to_string()))?;

        let label = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
        Ok(sid_last_subauthority(&label.Label))
    }
}

unsafe fn sid_last_subauthority(sid: &SID_AND_ATTRIBUTES) -> u32 {
    use windows::Win32::Security::{GetSidSubAuthority, GetSidSubAuthorityCount};
    let count = *GetSidSubAuthorityCount(sid.Sid);
    if count == 0 {
        return 0;
    }
    *GetSidSubAuthority(sid.Sid, (count - 1) as u32)
}

/// 依標題找一個可見的頂層視窗。**完全相符優先**，沒有才退回子字串比對。
///
/// 完全相符優先是必要的：主控台視窗的標題是完整執行檔路徑，
/// 子字串比對會讓 `find_window("inkkeep")` 抓到
/// `...\inkkeep\target\debug\examples\scratch_window.exe` 這種視窗。
pub fn find_window(title_substring: &str) -> Option<FocusToken> {
    use windows::Win32::Foundation::{BOOL, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, IsWindowVisible};

    struct Search {
        needle: String,
        exact: Option<FocusToken>,
        partial: Option<FocusToken>,
    }

    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = &mut *(lparam.0 as *mut Search);
        if !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }
        let title = window_title(hwnd);
        if title.is_empty() || !title.contains(&search.needle) {
            return BOOL(1);
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return BOOL(1);
        }
        let exact = title == search.needle;
        let token = FocusToken {
            hwnd: hwnd.0 as isize,
            pid,
            title,
        };
        if exact {
            search.exact = Some(token);
            return BOOL(0); // 完全相符就不必再找
        }
        if search.partial.is_none() {
            search.partial = Some(token);
        }
        BOOL(1) // 繼續找，看有沒有完全相符的
    }

    let mut search = Search {
        needle: title_substring.to_string(),
        exact: None,
        partial: None,
    };
    unsafe {
        let _ = EnumWindows(Some(callback), LPARAM(&mut search as *mut Search as isize));
    }
    search.exact.or(search.partial)
}

fn window_title(hwnd: HWND) -> String {
    unsafe {
        let mut buf = [0u16; 512];
        let len = GetWindowTextW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..len.max(0) as usize])
    }
}
