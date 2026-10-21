//! 钥匙串访问（FR-4.2、NFR-3）。应用侧唯一碰 Key 的地方。
//!
//! 对外只暴露三件事：Key 存不存在、存的时候成功没有、以及给引擎用的原文。
//! 任何返回值、日志、界面文案都不得包含 Key 的可还原片段——包括末 4 位。

use std::hash::{Hash, Hasher};

use ice_core::config::{KEYCHAIN_ACCOUNT, KEYCHAIN_SERVICE};

use crate::state::KeyState;

/// 只用于界面显示"是哪一份 Key"的指纹，不承担完整性或安全校验，所以用标准库默认哈希即可。
pub fn fingerprint(key: &str) -> (usize, String) {
    let mut hasher = std::hash::DefaultHasher::new();
    key.hash(&mut hasher);
    let digest = format!("{:016x}", hasher.finish());
    (key.chars().count(), digest[..8].to_string())
}

/// 把 keyring 的错误压成一句可显示的话。
///
/// `BadEncoding` / `BadDataFormat` 两个变体内部携带原始字节，`Display` 会把它们打出来，
/// 所以这里对这两个变体只说"格式不可读"，不转 Display、不写日志。
pub fn describe_error(err: &keyring::Error) -> String {
    match err {
        keyring::Error::NoEntry => "钥匙串里没有这条记录".to_string(),
        keyring::Error::BadEncoding(_) | keyring::Error::BadDataFormat(..) => {
            "钥匙串条目不是可读文本，请在设置页重新保存一次".to_string()
        }
        keyring::Error::NoStorageAccess(_) | keyring::Error::PlatformFailure(_) => {
            "系统拒绝了钥匙串访问，请检查是否允许 Ice Translator 使用钥匙串".to_string()
        }
        keyring::Error::TooLong(_, limit) => format!("Key 超过钥匙串长度上限（{limit} 字节）"),
        other => format!("钥匙串操作失败：{other}"),
    }
}

fn entry() -> Result<keyring::Entry, KeyState> {
    keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
        .map_err(|e| KeyState::Error(describe_error(&e)))
}

/// 读取 Key 原文。未配置返回 `None`；读取本身出错时打日志但不把细节带回调用方以外的地方。
pub fn read() -> Result<Option<String>, KeyState> {
    let entry = entry()?;
    match entry.get_password() {
        Ok(key) if !key.is_empty() => Ok(Some(key)),
        Ok(_) => Ok(None),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(KeyState::Error(describe_error(&err))),
    }
}

/// 探测配置状态，用于设置页显示"已配置（长度 N，指纹 xxxxxxxx）"。
pub fn probe() -> KeyState {
    match read() {
        Ok(Some(key)) => {
            let (len, hash8) = fingerprint(&key);
            KeyState::Configured { len, hash8 }
        }
        Ok(None) => KeyState::Missing,
        Err(state) => state,
    }
}

pub fn write(key: &str) -> Result<(), KeyState> {
    let entry = entry()?;
    entry
        .set_password(key)
        .map_err(|e| KeyState::Error(describe_error(&e)))
}

/// 删除条目。本来就不存在时按成功处理——用户的目标是"钥匙串里没有它"。
pub fn delete() -> Result<(), KeyState> {
    let entry = entry()?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(KeyState::Error(describe_error(&err))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable_and_hides_the_key() {
        let key = "sk-test-1234567890abcdef";
        let (len, hash8) = fingerprint(key);
        assert_eq!(len, key.chars().count());
        assert_eq!(hash8.len(), 8);
        assert_eq!(fingerprint(key), (len, hash8.clone()));
        assert_ne!(fingerprint("sk-test-1234567890abcdeg").1, hash8);
        // 指纹本身不得包含 Key 的任何片段
        assert!(!hash8.contains("1234"));
    }

    #[test]
    fn describe_error_never_echoes_payload_bytes() {
        // BadEncoding 携带原始字节；Display 会打出来，我们必须绕开
        let payload = b"sk-secret-payload-do-not-leak".to_vec();
        let text = describe_error(&keyring::Error::BadEncoding(payload.clone()));
        assert!(!text.contains("secret"));
        assert!(!text.contains("sk-"));

        let text = describe_error(&keyring::Error::TooLong("x".into(), 1024));
        assert!(text.contains("1024"));

        let text = describe_error(&keyring::Error::NoEntry);
        assert!(text.contains("没有"));
    }

    #[test]
    fn describe_error_covers_access_failures() {
        let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let text = describe_error(&keyring::Error::NoStorageAccess(Box::new(io)));
        assert!(text.contains("钥匙串"));
    }
}
