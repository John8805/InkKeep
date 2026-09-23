//! 送到前端的錯誤型別。

use serde::Serialize;
use inkkeep_core::model::ValidationError;
use inkkeep_core::template::TemplateError;
use inkkeep_core::vault::VaultError;
use inkkeep_win::WinError;

#[derive(Debug, Serialize)]
#[serde(tag = "kind", content = "detail")]
pub enum AppError {
    /// 保險庫沒開——沒有快取金鑰，要主密碼
    Locked,
    /// 保險庫開著，但第二層金鑰還沒推導，密碼項目仍是密文
    SecretsLocked,
    WrongPassword,
    NotFound {
        id: String,
    },
    Validation(Vec<ValidationError>),
    Template(TemplateError),
    VaultIo {
        path: String,
        message: String,
    },
    /// 寫入後驗證不一致，正本未被覆蓋
    VaultVerifyFailed {
        candidate: String,
        detail: String,
    },
    /// 選的檔案不是密碼匯出檔（找不到密碼欄）
    NotPasswordExport,
    WorkspaceNotFound,
    /// 工作區裡還有項目，不能刪
    WorkspaceNotEmpty {
        count: usize,
    },
    /// 最後一個工作區不能刪
    LastWorkspace,
    /// `empty`、`too-long`、`duplicate`
    WorkspaceName {
        reason: String,
    },
    /// 送出失敗；`clipboard` 表示內容是否已留在剪貼簿
    SendFailed {
        reason: String,
        clipboard: bool,
    },
    Other {
        message: String,
    },
}

impl From<VaultError> for AppError {
    fn from(e: VaultError) -> Self {
        match e {
            VaultError::WrongPassword => AppError::WrongPassword,
            VaultError::NotFound(p) => AppError::VaultIo {
                path: p.display().to_string(),
                message: "not-found".into(),
            },
            VaultError::Io { path, source } => AppError::VaultIo {
                path: path.display().to_string(),
                message: source.to_string(),
            },
            VaultError::VerifyFailed { candidate, detail } => AppError::VaultVerifyFailed {
                candidate: candidate.display().to_string(),
                detail,
            },
            VaultError::Kdbx(m) => AppError::Other { message: m },
            VaultError::NoSecretKeyMaterial => AppError::Other {
                message: "vault-missing-secret-material".into(),
            },
            VaultError::Crypto(e) => AppError::from(e),
            VaultError::WorkspaceNotFound => AppError::WorkspaceNotFound,
            VaultError::WorkspaceNotEmpty(count) => AppError::WorkspaceNotEmpty { count },
            VaultError::LastWorkspace => AppError::LastWorkspace,
            VaultError::WorkspaceName(reason) => AppError::WorkspaceName {
                reason: reason.into(),
            },
        }
    }
}

impl From<inkkeep_core::crypto::CryptoError> for AppError {
    fn from(e: inkkeep_core::crypto::CryptoError) -> Self {
        use inkkeep_core::crypto::CryptoError;
        match e {
            // 密文解不開最可能的原因是金鑰不對
            CryptoError::Decrypt => AppError::WrongPassword,
            other => AppError::Other {
                message: other.to_string(),
            },
        }
    }
}

impl From<Vec<ValidationError>> for AppError {
    fn from(e: Vec<ValidationError>) -> Self {
        AppError::Validation(e)
    }
}

impl From<TemplateError> for AppError {
    fn from(e: TemplateError) -> Self {
        AppError::Template(e)
    }
}

impl From<WinError> for AppError {
    fn from(e: WinError) -> Self {
        AppError::Other {
            message: e.to_string(),
        }
    }
}

impl From<inkkeep_core::import::ImportError> for AppError {
    fn from(e: inkkeep_core::import::ImportError) -> Self {
        use inkkeep_core::import::ImportError;
        match e {
            ImportError::NotFound(p) => AppError::VaultIo {
                path: p.display().to_string(),
                message: "bookmarks-not-found".into(),
            },
            other => AppError::Other {
                message: other.to_string(),
            },
        }
    }
}

impl From<inkkeep_core::gen::GenError> for AppError {
    fn from(e: inkkeep_core::gen::GenError) -> Self {
        AppError::Other {
            message: e.to_string(),
        }
    }
}
