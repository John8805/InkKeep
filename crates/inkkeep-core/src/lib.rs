//! inkkeep 核心邏輯：資料模型、搜尋、模板、密碼產生、kdbx 存取。
//! 不碰 OS 與 UI。

pub mod crypto;
pub mod gen;
pub mod import;
pub mod model;
pub mod password_import;
pub mod samples;
pub mod search;
pub mod template;
pub mod vault;

pub use model::{Item, ItemKind};
