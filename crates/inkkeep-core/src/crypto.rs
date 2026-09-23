//! 兩層金鑰：推導、密碼欄位的加解密。
//!
//! 主密碼跑兩次不同 salt 的 Argon2id，得到兩把互相推導不出來的金鑰：
//!
//! - **第一層**（[`derive_vault_key`]）當作 kdbx 檔案本身的密碼。算出來之後存進
//!   Windows 認證管理員，開 app 不必再問，片語與書籤立刻可用。
//! - **第二層**（[`derive_secret_key`]）管密碼項目的 Password 欄位。
//!   不存任何地方，每個 session 需要時重新推導。
//!
//! 兩層都從同一組主密碼來，所以換裝置只要記得主密碼，不必另外抄一串金鑰。
//!
//! # 為什麼密碼欄位走非對稱加密
//!
//! 第二層金鑰不直接拿來對稱加密，而是推出一對 X25519 金鑰：
//!
//! - **公鑰**存在保險庫裡（第一層就讀得到）。新增或修改密碼只需要公鑰，
//!   [`seal`] 不必問主密碼。
//! - **私鑰**每次從主密碼重新推導，不落地。要讀回密碼、送出密碼才需要它。
//!
//! 這把「寫」和「讀」的門檻拆開了：整理一堆帳號密碼的時候不必解鎖，
//! 真的要用某一筆的時候才輸入一次主密碼。
//!
//! 代價講清楚：**拿得到第一層金鑰的人可以新增或覆蓋密碼項目**，這是公鑰加密
//! 的本質，他讀不到你既有的密碼。能做到這件事的人本來就能刪檔或改片語，
//! 不是新增的攻擊面。

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chacha20poly1305::aead::Aead;
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use rand::RngCore;
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop};

// ---------- 檔案格式識別字 ----------
//
// 以下幾個常數會寫進保險庫檔案，或參與金鑰推導，**一旦有資料就永遠不能改**，
// 也不跟著產品名稱走（所以仍帶 `snipkit`）。改了之後：
// 第一層金鑰推不出同一把、既有密碼拆不開、驗證碼對不上。

/// 第一層的 salt 寫死在程式裡。
///
/// 它必須在**開檔之前**就拿得到，而且跨裝置要算出同一個值。能滿足這兩點的位置只有
/// 三個：kdbx 檔頭、保險庫旁邊的附屬檔、程式常數。
///
/// - 檔頭的 KDF seed 不行：`keepass` crate 每次存檔都重新產生一顆
///   （`format/kdbx4/dump.rs` 的 `get_kdf_and_seed`），存完一次金鑰就對不上了。
///   檔頭的 public custom data 則沒有公開的寫入介面。
/// - 附屬檔不行：弄丟它等於保險庫開不了，多一個致命失敗點。
///
/// 代價是拿到快取金鑰的人可以用預先算好的表反推主密碼。要先突破 Windows 登入
/// 與 DPAPI 才拿得到快取，而且每猜一次仍要跑一輪 64 MiB 的 Argon2id。
const VAULT_KEY_SALT: &[u8] = b"snipkit.vault-key.v1";

const MEM_COST_KIB: u32 = 64 * 1024;
const TIME_COST: u32 = 3;
const LANES: u32 = 4;
const KEY_LEN: usize = 32;

/// 第二層 salt 的長度。隨保險庫產生，存在 kdbx CustomData。
pub const SECRET_SALT_LEN: usize = 16;

/// 對稱密文的前綴。
const ENVELOPE: &str = "snipkit-enc:v1:";

/// 公鑰密文的前綴。
const SEALED: &str = "snipkit-sealed:v1:";

/// X25519 公鑰的長度。
pub const PUBLIC_KEY_LEN: usize = 32;

/// 從第二層金鑰推 X25519 私鑰用的標籤。換掉它等於換一把金鑰對。
const X25519_INFO: &[u8] = b"snipkit/x25519/v1";

/// 每一筆密文推 AEAD 金鑰用的標籤。
const SEAL_INFO: &[u8] = b"snipkit/seal/v1";

/// 驗證碼的明文。解得開就代表第二層金鑰正確。
const VERIFIER_PLAINTEXT: &str = "snipkit-secret-ok";

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("金鑰推導失敗：{0}")]
    Kdf(String),
    /// 密文被改過、截斷，或金鑰不對。AEAD 分不出這三者。
    #[error("內容解不開")]
    Decrypt,
    #[error("密文格式不對")]
    Malformed,
}

/// 第二層金鑰。
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecretKey([u8; KEY_LEN]);

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretKey(<redacted>)")
    }
}

fn argon2id(password: &str, salt: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let config = argon2::Config {
        variant: argon2::Variant::Argon2id,
        version: argon2::Version::Version13,
        mem_cost: MEM_COST_KIB,
        time_cost: TIME_COST,
        lanes: LANES,
        hash_length: KEY_LEN as u32,
        ..argon2::Config::default()
    };
    argon2::hash_raw(password.as_bytes(), salt, &config)
        .map_err(|e| CryptoError::Kdf(e.to_string()))
}

/// 第一層：kdbx 檔案的密碼。
///
/// 回傳的字串可以直接貼進 KeePassXC 開檔。
pub fn derive_vault_key(password: &str) -> Result<String, CryptoError> {
    Ok(B64.encode(argon2id(password, VAULT_KEY_SALT)?))
}

/// 第二層：密碼欄位的加密金鑰。
pub fn derive_secret_key(password: &str, salt: &[u8]) -> Result<SecretKey, CryptoError> {
    let mut raw = argon2id(password, salt)?;
    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(&raw);
    raw.zeroize();
    Ok(SecretKey(key))
}

pub fn random_salt() -> Vec<u8> {
    let mut salt = vec![0u8; SECRET_SALT_LEN];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    salt
}

/// 已經是我們的密文格式。
pub fn is_encrypted(text: &str) -> bool {
    text.starts_with(ENVELOPE) || text.starts_with(SEALED)
}

// ---------- 公鑰：存密碼不必解鎖 ----------

/// 第二層金鑰對應的 X25519 私鑰。
fn x25519_secret(key: &SecretKey) -> StaticSecret {
    let hk = Hkdf::<Sha256>::new(None, &key.0);
    let mut raw = [0u8; 32];
    // 只有輸出長度超過 255×32 bytes 才會失敗，32 不可能
    hk.expand(X25519_INFO, &mut raw).expect("HKDF 輸出長度合法");
    let secret = StaticSecret::from(raw);
    raw.zeroize();
    secret
}

/// 這把主密碼對應的公鑰。
pub fn public_key(key: &SecretKey) -> [u8; PUBLIC_KEY_LEN] {
    PublicKey::from(&x25519_secret(key)).to_bytes()
}

/// 每筆密文的 AEAD 金鑰：把共享祕密與雙方公鑰一起餵進 HKDF。
///
/// 綁上兩把公鑰，金鑰就只在這一組收送件人之間有效。
fn seal_key(shared: &[u8], ephemeral: &[u8], recipient: &[u8]) -> [u8; KEY_LEN] {
    let salt = [ephemeral, recipient].concat();
    let hk = Hkdf::<Sha256>::new(Some(&salt), shared);
    let mut key = [0u8; KEY_LEN];
    hk.expand(SEAL_INFO, &mut key).expect("HKDF 輸出長度合法");
    key
}

/// 用公鑰封一段文字。
pub fn seal(recipient: &[u8; PUBLIC_KEY_LEN], plaintext: &str) -> Result<String, CryptoError> {
    let mut raw = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    let ephemeral = StaticSecret::from(raw);
    raw.zeroize();

    let ephemeral_pk = PublicKey::from(&ephemeral).to_bytes();
    let shared = ephemeral.diffie_hellman(&PublicKey::from(*recipient));
    let mut key = seal_key(shared.as_bytes(), &ephemeral_pk, recipient);

    let mut nonce = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let cipher = XChaCha20Poly1305::new(key.as_slice().into());
    let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), plaintext.as_bytes())
        .map_err(|_| CryptoError::Decrypt)?;
    key.zeroize();

    let mut blob = Vec::with_capacity(ephemeral_pk.len() + nonce.len() + ciphertext.len());
    blob.extend_from_slice(&ephemeral_pk);
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&ciphertext);
    Ok(format!("{SEALED}{}", B64.encode(blob)))
}

/// 拆開 [`seal`] 封起來的文字。
///
/// 也接受 [`encrypt`] 的對稱密文；既有保險庫裡的密碼欄位可能仍是這種格式。
pub fn unseal(key: &SecretKey, blob: &str) -> Result<String, CryptoError> {
    let Some(body) = blob.strip_prefix(SEALED) else {
        return decrypt(key, blob);
    };
    let raw = B64.decode(body).map_err(|_| CryptoError::Malformed)?;
    if raw.len() < PUBLIC_KEY_LEN + 24 {
        return Err(CryptoError::Malformed);
    }
    let (ephemeral_pk, rest) = raw.split_at(PUBLIC_KEY_LEN);
    let (nonce, ciphertext) = rest.split_at(24);

    let secret = x25519_secret(key);
    let recipient = PublicKey::from(&secret).to_bytes();
    let mut eph = [0u8; PUBLIC_KEY_LEN];
    eph.copy_from_slice(ephemeral_pk);
    let shared = secret.diffie_hellman(&PublicKey::from(eph));
    let mut aead_key = seal_key(shared.as_bytes(), ephemeral_pk, &recipient);

    let cipher = XChaCha20Poly1305::new(aead_key.as_slice().into());
    let plain = cipher
        .decrypt(XNonce::from_slice(nonce), ciphertext)
        .map_err(|_| CryptoError::Decrypt)?;
    aead_key.zeroize();
    String::from_utf8(plain).map_err(|_| CryptoError::Decrypt)
}

pub fn encrypt(key: &SecretKey, plaintext: &str) -> Result<String, CryptoError> {
    let cipher = XChaCha20Poly1305::new(key.0.as_slice().into());
    let mut nonce = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut nonce);

    let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), plaintext.as_bytes())
        .map_err(|_| CryptoError::Decrypt)?;

    let mut blob = Vec::with_capacity(nonce.len() + ciphertext.len());
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&ciphertext);
    Ok(format!("{ENVELOPE}{}", B64.encode(blob)))
}

pub fn decrypt(key: &SecretKey, blob: &str) -> Result<String, CryptoError> {
    let body = blob.strip_prefix(ENVELOPE).ok_or(CryptoError::Malformed)?;
    let raw = B64.decode(body).map_err(|_| CryptoError::Malformed)?;
    if raw.len() < 24 {
        return Err(CryptoError::Malformed);
    }
    let (nonce, ciphertext) = raw.split_at(24);

    let cipher = XChaCha20Poly1305::new(key.0.as_slice().into());
    let plain = cipher
        .decrypt(XNonce::from_slice(nonce), ciphertext)
        .map_err(|_| CryptoError::Decrypt)?;
    String::from_utf8(plain).map_err(|_| CryptoError::Decrypt)
}

/// 產生驗證碼，之後用來判斷使用者輸入的主密碼對不對。
pub fn make_verifier(key: &SecretKey) -> Result<String, CryptoError> {
    encrypt(key, VERIFIER_PLAINTEXT)
}

pub fn check_verifier(key: &SecretKey, verifier: &str) -> bool {
    matches!(decrypt(key, verifier), Ok(text) if text == VERIFIER_PLAINTEXT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 這些字串寫進了使用者的保險庫檔案或參與金鑰推導，改了就讀不回舊資料。
    /// 它們刻意不跟著產品名稱走，全域取代產品名稱時要避開。
    #[test]
    fn frozen_format_constants() {
        assert_eq!(VAULT_KEY_SALT, b"snipkit.vault-key.v1");
        assert_eq!(ENVELOPE, "snipkit-enc:v1:");
        assert_eq!(SEALED, "snipkit-sealed:v1:");
        assert_eq!(X25519_INFO, b"snipkit/x25519/v1");
        assert_eq!(SEAL_INFO, b"snipkit/seal/v1");
        assert_eq!(VERIFIER_PLAINTEXT, "snipkit-secret-ok");
    }

    /// 已知答案：同一組主密碼永遠推出同一把第一層金鑰。
    /// 換裝置時就是靠這個開得了同一個檔案——任何改動推導方式的修改都會讓它失敗。
    #[test]
    fn vault_key_known_answer() {
        assert_eq!(
            derive_vault_key("correct horse battery staple").unwrap(),
            "TYwIIa5KPJldNDH19rNni9VzDll8wDzCaumtKuZt1KU="
        );
    }

    /// 已知答案：同一把第二層金鑰永遠推出同一把公鑰，否則既有的密碼拆不開。
    #[test]
    fn public_key_known_answer() {
        assert_eq!(
            B64.encode(public_key(&SecretKey([7u8; KEY_LEN]))),
            "a3wOs/pFVhEaOGCl4QJpTnBdiJjThDo7Z4SyDxvbfWs="
        );
    }

    fn test_key(seed: u8) -> SecretKey {
        SecretKey([seed; KEY_LEN])
    }

    #[test]
    fn round_trip() {
        let key = test_key(1);
        let blob = encrypt(&key, "hunter2 中文 \u{1F600}").unwrap();
        assert!(is_encrypted(&blob));
        assert_eq!(decrypt(&key, &blob).unwrap(), "hunter2 中文 \u{1F600}");
    }

    #[test]
    fn wrong_key_fails_to_decrypt() {
        let blob = encrypt(&test_key(1), "hunter2").unwrap();
        assert!(matches!(
            decrypt(&test_key(2), &blob),
            Err(CryptoError::Decrypt)
        ));
    }

    /// 同一段明文每次加密都要不一樣，否則相同的密碼在檔案裡看得出來是同一個。
    #[test]
    fn nonce_is_fresh_each_time() {
        let key = test_key(1);
        let a = encrypt(&key, "same").unwrap();
        let b = encrypt(&key, "same").unwrap();
        assert_ne!(a, b);
    }

    /// 改動密文任何一個位元組都要被 AEAD 擋下來。
    #[test]
    fn tampering_is_detected() {
        let key = test_key(1);
        let blob = encrypt(&key, "hunter2").unwrap();
        let body = blob.strip_prefix(ENVELOPE).unwrap();
        let mut raw = B64.decode(body).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0x01;
        let tampered = format!("{ENVELOPE}{}", B64.encode(&raw));
        assert!(matches!(
            decrypt(&key, &tampered),
            Err(CryptoError::Decrypt)
        ));
    }

    #[test]
    fn plaintext_is_not_mistaken_for_ciphertext() {
        assert!(!is_encrypted("hunter2"));
        assert!(matches!(
            decrypt(&test_key(1), "hunter2"),
            Err(CryptoError::Malformed)
        ));
    }

    /// 這是整套設計的重點：封的時候只要公鑰，拆的時候才要主密碼推出來的金鑰。
    #[test]
    fn sealing_needs_only_the_public_key() {
        let key = test_key(1);
        let pk = public_key(&key);

        let blob = seal(&pk, "hunter2 中文").unwrap();
        assert!(is_encrypted(&blob));
        assert!(!blob.contains("hunter2"));
        assert_eq!(unseal(&key, &blob).unwrap(), "hunter2 中文");
    }

    #[test]
    fn another_master_password_cannot_unseal() {
        let blob = seal(&public_key(&test_key(1)), "hunter2").unwrap();
        assert!(matches!(
            unseal(&test_key(2), &blob),
            Err(CryptoError::Decrypt)
        ));
    }

    /// 同一組主密碼一定要推出同一把公鑰，否則換裝置之後存的東西自己讀不回來。
    #[test]
    fn the_public_key_is_deterministic() {
        assert_eq!(public_key(&test_key(1)), public_key(&test_key(1)));
        assert_ne!(public_key(&test_key(1)), public_key(&test_key(2)));
    }

    /// 每次封都換一把臨時金鑰，相同的密碼在檔案裡看不出來是同一個。
    #[test]
    fn each_seal_uses_a_fresh_ephemeral_key() {
        let pk = public_key(&test_key(1));
        assert_ne!(seal(&pk, "same").unwrap(), seal(&pk, "same").unwrap());
    }

    #[test]
    fn a_tampered_seal_is_detected() {
        let key = test_key(1);
        let blob = seal(&public_key(&key), "hunter2").unwrap();
        let mut raw = B64.decode(blob.strip_prefix(SEALED).unwrap()).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0x01;
        assert!(matches!(
            unseal(&key, &format!("{SEALED}{}", B64.encode(&raw))),
            Err(CryptoError::Decrypt)
        ));
    }

    /// 既有保險庫裡的對稱密文仍然讀得回來。
    #[test]
    fn unseal_still_reads_the_old_symmetric_envelope() {
        let key = test_key(1);
        let old = encrypt(&key, "hunter2").unwrap();
        assert!(old.starts_with(ENVELOPE));
        assert_eq!(unseal(&key, &old).unwrap(), "hunter2");
    }

    #[test]
    fn verifier_accepts_only_its_own_key() {
        let v = make_verifier(&test_key(1)).unwrap();
        assert!(check_verifier(&test_key(1), &v));
        assert!(!check_verifier(&test_key(2), &v));
    }

    /// 兩層安全性完全靠 salt 分離：同一組主密碼、不同 salt 才推得出兩把不同的金鑰。
    /// 第二層的 salt 隨保險庫隨機產生，撞上第一層那顆常數的機率可以忽略。
    #[test]
    fn two_tiers_differ() {
        let vault_key = B64.decode(derive_vault_key("correct horse").unwrap()).unwrap();
        let secret = derive_secret_key("correct horse", &random_salt()).unwrap();
        assert_ne!(vault_key, secret.0.to_vec());

        // 反過來確認差異真的來自 salt，而不是兩條路徑剛好不同
        let same_salt = derive_secret_key("correct horse", VAULT_KEY_SALT).unwrap();
        assert_eq!(vault_key, same_salt.0.to_vec());
    }

    #[test]
    fn vault_key_is_deterministic() {
        assert_eq!(
            derive_vault_key("correct horse").unwrap(),
            derive_vault_key("correct horse").unwrap()
        );
        assert_ne!(
            derive_vault_key("correct horse").unwrap(),
            derive_vault_key("correct horsf").unwrap()
        );
    }
}
