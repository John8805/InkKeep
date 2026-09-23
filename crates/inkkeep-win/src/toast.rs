//! 讓 Windows 願意顯示這個程式的通知。
//!
//! Windows 的 toast 一定要掛在一個已註冊的 **AppUserModelID** 底下。
//!
//! 少了它，`tauri-plugin-notification` 的 `show()` 仍然回傳 Ok，但通知不會出現。
//!
//! 寫的位置是 `HKCU\SOFTWARE\Classes\AppUserModelId\<app_id>`：只影響目前使用者、
//! 不需要管理員權限，刪掉也只是讓通知消失而已。

use crate::WinError;
use windows::core::PCWSTR;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
    REG_OPTION_NON_VOLATILE, REG_SZ,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 註冊 AppUserModelID。已經註冊過就是覆寫同樣的值，重複呼叫沒有副作用。
///
/// `display_name` 是通知右上角顯示的程式名稱。
pub fn register_app_id(app_id: &str, display_name: &str) -> Result<(), WinError> {
    let subkey = wide(&format!("SOFTWARE\\Classes\\AppUserModelId\\{app_id}"));
    let value_name = wide("DisplayName");
    let display = wide(display_name);
    // REG_SZ 要的是位元組數，含結尾的 NUL
    let display_bytes =
        unsafe { std::slice::from_raw_parts(display.as_ptr() as *const u8, display.len() * 2) };

    let mut key = HKEY::default();
    // SAFETY: 所有指標都指向本函式仍持有的緩衝區
    let created = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            0,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
    };
    if created.is_err() {
        return Err(WinError::Other(format!(
            "建立註冊表項目失敗：{:?}",
            created.to_hresult()
        )));
    }

    // SAFETY: key 由上面成功建立，display_bytes 指向仍存活的緩衝區
    let set = unsafe {
        RegSetValueExW(
            key,
            PCWSTR(value_name.as_ptr()),
            0,
            REG_SZ,
            Some(display_bytes),
        )
    };
    // SAFETY: key 是上面拿到的有效 handle
    unsafe {
        let _ = RegCloseKey(key);
    }

    if set.is_err() {
        return Err(WinError::Other(format!(
            "寫入 DisplayName 失敗：{:?}",
            set.to_hresult()
        )));
    }
    Ok(())
}

// ---------- 開始選單捷徑 ----------

/// 在開始選單建一個帶 AppUserModelID 的捷徑。已存在就覆寫。
///
/// 覆寫而不是「有就跳過」：執行檔搬過位置之後舊捷徑會指向不存在的路徑，
/// 而失效的捷徑等於通知不會出現，這種壞法完全沒有徵兆。
///
/// 光寫註冊表的 `DisplayName` 不夠，Windows 仍然會把 toast 丟掉。Windows 認的是
/// 「開始選單裡有一個捷徑，而且它的 `System.AppUserModel.ID` 屬性等於送出通知
/// 時用的 app id」。
///
/// 建在 `%APPDATA%\Microsoft\Windows\Start Menu\Programs\`，只影響目前使用者，
/// 刪掉也只是讓通知消失。
pub fn ensure_start_menu_shortcut(app_id: &str, display_name: &str) -> Result<(), WinError> {
    use windows::core::Interface;
    use windows::core::PROPVARIANT;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, PROPERTYKEY};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

    let exe = std::env::current_exe()
        .map_err(|e| WinError::Other(format!("取不到執行檔路徑：{e}")))?;
    let appdata = std::env::var("APPDATA")
        .map_err(|_| WinError::Other("環境變數 APPDATA 不存在".into()))?;
    let link_path = std::path::Path::new(&appdata)
        .join(r"Microsoft\Windows\Start Menu\Programs")
        .join(format!("{display_name}.lnk"));

    let exe_w = wide(&exe.to_string_lossy());
    let link_w = wide(&link_path.to_string_lossy());

    // SAFETY: 全部指標都指向本函式持有的緩衝區；COM 物件由 windows crate 管生命週期
    unsafe {
        // 執行緒可能已經初始化過 COM，重複呼叫回 S_FALSE 或
        // RPC_E_CHANGED_MODE，兩者都不影響後面的操作
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| WinError::Other(format!("建立 ShellLink 失敗：{e}")))?;
        link.SetPath(PCWSTR(exe_w.as_ptr()))
            .map_err(|e| WinError::Other(format!("設定捷徑目標失敗：{e}")))?;

        // System.AppUserModel.ID
        const PKEY_APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
            fmtid: windows::core::GUID::from_u128(0x9F4C2855_9F79_4B39_A8D0_E1D42DE1D5F3),
            pid: 5,
        };
        let store: IPropertyStore = link
            .cast()
            .map_err(|e| WinError::Other(format!("取得屬性儲存體失敗：{e}")))?;
        store
            .SetValue(&PKEY_APP_USER_MODEL_ID, &PROPVARIANT::from(app_id))
            .map_err(|e| WinError::Other(format!("設定 AppUserModelID 失敗：{e}")))?;
        store
            .Commit()
            .map_err(|e| WinError::Other(format!("寫入屬性失敗：{e}")))?;

        let file: IPersistFile = link
            .cast()
            .map_err(|e| WinError::Other(format!("取得 IPersistFile 失敗：{e}")))?;
        file.Save(PCWSTR(link_w.as_ptr()), true)
            .map_err(|e| WinError::Other(format!("存檔捷徑失敗：{e}")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 這個測試會真的寫進目前使用者的註冊表，所以用一個帶隨機字尾的假 app id，
    /// 而且自己收尾。
    #[test]
    fn registering_is_idempotent() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let app_id = format!("app.inkkeep.test.{nanos:x}");

        register_app_id(&app_id, "inkkeep test").expect("第一次註冊");
        register_app_id(&app_id, "inkkeep test").expect("重複註冊不該出錯");

        let subkey = wide(&format!("SOFTWARE\\Classes\\AppUserModelId\\{app_id}"));
        unsafe {
            let _ = windows::Win32::System::Registry::RegDeleteKeyW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
            );
        }
    }
}
