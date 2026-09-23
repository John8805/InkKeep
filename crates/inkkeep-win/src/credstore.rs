//! Windows 認證管理員。存第一層金鑰。
//!
//! 寫進去的 blob 由 DPAPI 以使用者的登入憑證保護：換一個 Windows 帳號讀不出來，
//! 把硬碟拔到別台機器也讀不出來。
//!
//! 每個保險庫路徑各存一份。

use crate::WinError;
use std::path::Path;
use windows::core::PWSTR;
use windows::Win32::Foundation::ERROR_NOT_FOUND;
use windows::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_FLAGS,
    CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
};

/// 認證管理員裡顯示的名稱。
fn target_name(vault_path: &Path) -> String {
    format!("inkkeep:vault:{}", vault_path.display())
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 存起來。同一個保險庫再存一次會覆蓋。
pub fn store(vault_path: &Path, secret: &str) -> Result<(), WinError> {
    let mut target = wide(&target_name(vault_path));
    let mut user = wide("inkkeep");
    let mut blob = secret.as_bytes().to_vec();
    if blob.len() > CRED_MAX_CREDENTIAL_BLOB_SIZE as usize {
        return Err(WinError::Other(format!(
            "金鑰長度 {} 超過認證管理員上限 {CRED_MAX_CREDENTIAL_BLOB_SIZE}",
            blob.len()
        )));
    }

    let cred = CREDENTIALW {
        Flags: CRED_FLAGS(0),
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target.as_mut_ptr()),
        Comment: PWSTR::null(),
        LastWritten: Default::default(),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        AttributeCount: 0,
        Attributes: std::ptr::null_mut(),
        TargetAlias: PWSTR::null(),
        UserName: PWSTR(user.as_mut_ptr()),
    };

    // SAFETY: cred 裡的三個指標都指向本函式仍持有的緩衝區，CredWriteW 只在呼叫期間讀它們
    unsafe { CredWriteW(&cred, 0) }.map_err(|e| WinError::Other(e.to_string()))
}

/// 讀回來。沒存過回 `Ok(None)`，其他失敗才是 `Err`。
pub fn load(vault_path: &Path) -> Result<Option<String>, WinError> {
    let target = wide(&target_name(vault_path));
    let mut ptr: *mut CREDENTIALW = std::ptr::null_mut();

    // SAFETY: target 是有結尾 NUL 的寬字串，ptr 由 CredReadW 填入
    let result = unsafe { CredReadW(PWSTR(target.as_ptr() as *mut u16), CRED_TYPE_GENERIC, 0, &mut ptr) };
    if let Err(e) = result {
        if e.code() == ERROR_NOT_FOUND.to_hresult() {
            return Ok(None);
        }
        return Err(WinError::Other(e.to_string()));
    }
    if ptr.is_null() {
        return Ok(None);
    }

    // SAFETY: CredReadW 成功且 ptr 非空，緩衝區在 CredFree 之前有效
    let secret = unsafe {
        let cred = &*ptr;
        let bytes =
            std::slice::from_raw_parts(cred.CredentialBlob, cred.CredentialBlobSize as usize);
        let text = String::from_utf8(bytes.to_vec());
        CredFree(ptr as *const _);
        text
    };

    // 讀出來不是 UTF-8 就當它壞了，視同沒存過
    Ok(secret.ok())
}

/// 刪掉。沒存過不算錯。
pub fn forget(vault_path: &Path) -> Result<(), WinError> {
    let target = wide(&target_name(vault_path));
    // SAFETY: target 是有結尾 NUL 的寬字串
    match unsafe { CredDeleteW(PWSTR(target.as_ptr() as *mut u16), CRED_TYPE_GENERIC, 0) } {
        Ok(()) => Ok(()),
        Err(e) if e.code() == ERROR_NOT_FOUND.to_hresult() => Ok(()),
        Err(e) => Err(WinError::Other(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 這些測試會真的寫進目前使用者的認證管理員，所以用一個帶隨機字尾的假路徑，
    /// 而且每個測試自己收尾——不能碰到使用者實際保險庫那一筆。
    fn scratch_path() -> PathBuf {
        PathBuf::from(format!(
            "C:\\inkkeep-test-{}\\vault.kdbx",
            uuid_like_suffix()
        ))
    }

    fn uuid_like_suffix() -> String {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};
        // 時間戳加序號：同一個測試裡連呼叫兩次也保證不同
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("{nanos:x}-{}", SEQ.fetch_add(1, Ordering::Relaxed))
    }

    #[test]
    fn store_load_forget_round_trip() {
        let path = scratch_path();
        assert_eq!(load(&path).unwrap(), None, "一開始不該有東西");

        store(&path, "dGhpcyBpcyBhIGtleQ==").unwrap();
        assert_eq!(load(&path).unwrap().as_deref(), Some("dGhpcyBpcyBhIGtleQ=="));

        store(&path, "second").unwrap();
        assert_eq!(load(&path).unwrap().as_deref(), Some("second"), "要能覆蓋");

        forget(&path).unwrap();
        assert_eq!(load(&path).unwrap(), None);
        forget(&path).unwrap_or_else(|e| panic!("重複刪除不該出錯：{e}"));
    }

    /// 兩個保險庫各自一把金鑰，不能互相蓋掉。
    #[test]
    fn different_vaults_do_not_collide() {
        let a = scratch_path();
        let b = scratch_path();
        store(&a, "key-a").unwrap();
        store(&b, "key-b").unwrap();
        assert_eq!(load(&a).unwrap().as_deref(), Some("key-a"));
        assert_eq!(load(&b).unwrap().as_deref(), Some("key-b"));
        forget(&a).unwrap();
        assert_eq!(load(&b).unwrap().as_deref(), Some("key-b"), "刪 a 不該影響 b");
        forget(&b).unwrap();
    }
}
