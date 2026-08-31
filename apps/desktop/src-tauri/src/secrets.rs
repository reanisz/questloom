//! シークレット(資格情報)の保存先。
//!
//! questloom が扱うシークレットは **DB (`settings` テーブル) には置かない**。
//! 実体は OS の資格情報ストア(Windows なら資格情報マネージャー)で、
//! [`keyring`] crate 越しに読み書きする。DB に残るのは「設定済みかどうか」を
//! 示すものすら無く、値も痕跡も入らない。
//!
//! | シークレット | エントリ名 |
//! |---|---|
//! | 内蔵 MCP サーバーの Bearer トークン | `core/mcp-token` |
//! | プラグインの `type: "secret"` 項目 | `plugin:<id>/<key>` |
//!
//! service 名は既定で [`DEFAULT_SERVICE`](crate::secrets::DEFAULT_SERVICE) (`questloom`)。
//! テストや検証で本物のエントリを汚さないよう、`QUESTLOOM_KEYRING_SERVICE`
//! ([`crate::env_override`])で丸ごと差し替えられる。
//!
//! ## 抽象を挟む理由
//!
//! [`SecretStore`] という薄い trait を挟み、テストは [`MemorySecretStore`] を使う。
//! 資格情報マネージャーはプロセス・ユーザー単位でグローバルなので、単体テストが
//! 実エントリに触れると開発者の環境を壊しうる。実バックエンドを通すテストは
//! `#[ignore]` を付けた 1 本だけにしてある。
//!
//! ## 失敗したときの方針
//!
//! **平文へのフォールバックはしない。** ストアが使えない環境では読み書きとも
//! エラーにし、UI へそのまま伝える(保存できなかったことを利用者に見せる方が、
//! 黙って平文に落とすより安全)。唯一の例外は「以前のバージョンが DB に平文で
//! 保存した値の移送」で、これは移送に失敗しても既存の値を消さずに残し、
//! 次回起動でやり直す([`crate::state::AppState`])。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

/// 資格情報ストアに登録する既定の service 名。
pub const DEFAULT_SERVICE: &str = "questloom";

/// コア設定のシークレットに使う名前空間。
const CORE_NAMESPACE: &str = "core";

/// プラグインのシークレットに使う名前空間の接頭辞。
const PLUGIN_PREFIX: &str = "plugin:";

/// 内蔵 MCP サーバーの Bearer トークンのキー名(名前空間内)。
const MCP_TOKEN_KEY: &str = "mcp-token";

/// エントリ名の 1 区画に許す最大長。
///
/// Windows の資格情報マネージャーはターゲット名に 32767 文字まで許すので実質的な
/// 制限ではないが、事故(巨大な id を投げ込まれる)を避けるために切っておく。
const MAX_SEGMENT_LEN: usize = 128;

/// シークレットの読み書きに失敗したことを表す。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    /// 資格情報ストアそのものが使えない・操作に失敗した。
    #[error("資格情報マネージャーを利用できません ({key}): {message}")]
    Backend {
        /// 対象のエントリ名。
        key: String,
        /// バックエンドのメッセージ。
        message: String,
    },

    /// エントリ名に使えない文字列を渡された。
    #[error("シークレットのキー \"{segment}\" が不正です(英数字・-・_・. のみ、1〜{MAX_SEGMENT_LEN} 文字)。")]
    InvalidKey {
        /// 不正だった区画。
        segment: String,
    },
}

/// シークレット 1 件を指すエントリ名。
///
/// 生成は [`SecretKey::mcp_token`] / [`SecretKey::plugin`] のみ。名前空間の付け方を
/// 1 箇所に閉じ込め、外から任意の文字列でエントリを作れないようにするため。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SecretKey(String);

impl SecretKey {
    /// 内蔵 MCP サーバーの Bearer トークン (`core/mcp-token`)。
    #[must_use]
    pub fn mcp_token() -> Self {
        Self(format!("{CORE_NAMESPACE}/{MCP_TOKEN_KEY}"))
    }

    /// プラグインのシークレット項目 (`plugin:<id>/<key>`)。
    ///
    /// # Errors
    /// `plugin_id` / `key` に使えない文字が含まれる場合。
    pub fn plugin(plugin_id: &str, key: &str) -> Result<Self, SecretError> {
        validate_segment(plugin_id)?;
        validate_segment(key)?;
        Ok(Self(format!("{PLUGIN_PREFIX}{plugin_id}/{key}")))
    }

    /// エントリ名。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SecretKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// エントリ名の 1 区画として使えるか検査する。
///
/// 許すのは英数字・`-`・`_`・`.` のみ。プラグイン id は JS 側 (`sdk.ts` の
/// `ID_PATTERN`) でも同じ規則で弾かれるが、command の引数は webview から
/// 任意に渡せるので、境界であるここでも必ず見る(`/` や `..` を混ぜて別の
/// 名前空間のエントリを指させないため)。
fn validate_segment(segment: &str) -> Result<(), SecretError> {
    let ok = !segment.is_empty()
        && segment.len() <= MAX_SEGMENT_LEN
        // `.` / `..` はエントリ名としては無害だが、名前を扱う側(ログ・将来の
        // ファイル出力)で意味を持ちうるので念のため弾く。
        && !segment.chars().all(|c| c == '.')
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(())
    } else {
        Err(SecretError::InvalidKey {
            segment: segment.to_owned(),
        })
    }
}

/// シークレットの保存先。
///
/// 実体は [`KeyringSecretStore`]、テストは [`MemorySecretStore`]。
pub trait SecretStore: Send + Sync + std::fmt::Debug {
    /// 値を読む。未設定なら `Ok(None)`。
    ///
    /// # Errors
    /// バックエンドの操作に失敗した場合。
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError>;

    /// 値を書く(上書き)。
    ///
    /// # Errors
    /// バックエンドの操作に失敗した場合。
    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError>;

    /// 値を消す。未設定でも成功扱い(冪等)。
    ///
    /// # Errors
    /// バックエンドの操作に失敗した場合。
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError>;
}

/// 「値を設定する / 空なら消す」をまとめた補助。
///
/// UI から来る値は前後の空白を落とし、空文字列は「未設定」と読み替える
/// (MCP トークンの空白のみ = 認証なし、という従来の扱いに合わせる)。
/// 設定後の状態(設定済みなら `true`)を返す。
///
/// # Errors
/// バックエンドの操作に失敗した場合。
pub fn put(
    store: &dyn SecretStore,
    key: &SecretKey,
    value: Option<&str>,
) -> Result<bool, SecretError> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => {
            store.set(key, value)?;
            Ok(true)
        }
        None => {
            store.delete(key)?;
            Ok(false)
        }
    }
}

/// OS の資格情報ストアを使う本番の実装。
#[derive(Debug, Clone)]
pub struct KeyringSecretStore {
    service: String,
}

impl KeyringSecretStore {
    /// service 名を指定して作る。
    #[must_use]
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    /// この store が使う service 名。
    #[must_use]
    pub fn service(&self) -> &str {
        &self.service
    }

    fn entry(&self, key: &SecretKey) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(&self.service, key.as_str()).map_err(|error| backend(key, &error))
    }
}

/// [`keyring::Error`] を [`SecretError::Backend`] にする。
fn backend(key: &SecretKey, error: &keyring::Error) -> SecretError {
    SecretError::Backend {
        key: key.as_str().to_owned(),
        message: error.to_string(),
    }
}

impl SecretStore for KeyringSecretStore {
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError> {
        match self.entry(key)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(backend(key, &error)),
        }
    }

    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError> {
        self.entry(key)?
            .set_password(value)
            .map_err(|error| backend(key, &error))
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        match self.entry(key)?.delete_credential() {
            // 消えていることが目的なので、元から無いのは成功扱い。
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(backend(key, &error)),
        }
    }
}

/// プロセス内だけに持つ実装。テスト用。
///
/// 資格情報マネージャーはユーザー単位でグローバルなので、単体テストからは
/// 実バックエンドを触らない。
#[derive(Debug, Default)]
pub struct MemorySecretStore {
    entries: Mutex<BTreeMap<String, String>>,
    /// 真にすると全操作が [`SecretError::Backend`] になる(失敗経路のテスト用)。
    failing: bool,
}

impl MemorySecretStore {
    /// 空のストアを作る。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 何をしても失敗するストアを作る(ストアが使えない環境の再現)。
    #[must_use]
    pub fn failing() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
            failing: true,
        }
    }

    /// 保持しているエントリ名の一覧(昇順)。テストの確認用。
    #[must_use]
    pub fn keys(&self) -> Vec<String> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect()
    }

    fn guard(&self, key: &SecretKey) -> Result<(), SecretError> {
        if self.failing {
            return Err(SecretError::Backend {
                key: key.as_str().to_owned(),
                message: "テスト用の失敗するストア".to_owned(),
            });
        }
        Ok(())
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError> {
        self.guard(key)?;
        Ok(self
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key.as_str())
            .cloned())
    }

    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError> {
        self.guard(key)?;
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.as_str().to_owned(), value.to_owned());
        Ok(())
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.guard(key)?;
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key.as_str());
        Ok(())
    }
}

/// 本番で使うストアを組み立てる。
///
/// service 名は `QUESTLOOM_KEYRING_SERVICE` があればそれ、無ければ
/// [`DEFAULT_SERVICE`]([`crate::env_override::keyring_service`])。
#[must_use]
pub fn default_store() -> Arc<dyn SecretStore> {
    let service = crate::env_override::keyring_service();
    if service != DEFAULT_SERVICE {
        tracing::info!(
            service,
            "{} でシークレットの service 名を上書きします",
            crate::env_override::KEYRING_SERVICE_ENV
        );
    }
    Arc::new(KeyringSecretStore::new(service))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_namespaced() {
        assert_eq!(SecretKey::mcp_token().as_str(), "core/mcp-token");
        assert_eq!(
            SecretKey::plugin("github", "pat").unwrap().as_str(),
            "plugin:github/pat"
        );
        // MCP トークンとプラグインのキーは決して衝突しない。
        assert_ne!(
            SecretKey::plugin("core", "mcp-token").unwrap(),
            SecretKey::mcp_token()
        );
    }

    #[test]
    fn plugin_keys_reject_anything_that_could_escape_the_namespace() {
        for (id, key) in [
            ("", "pat"),
            ("github", ""),
            ("git/hub", "pat"),
            ("github", "pa/t"),
            ("..", "pat"),
            ("github", ".."),
            ("git\\hub", "pat"),
            ("core", "mcp token"),
            ("ギットハブ", "pat"),
            ("github", "pat\0"),
        ] {
            assert!(
                SecretKey::plugin(id, key).is_err(),
                "({id:?}, {key:?}) は拒否する"
            );
        }
        // `.` は許す(バージョン付きの id などを想定)。
        assert!(SecretKey::plugin("my.plugin", "api.key").is_ok());
        assert!(SecretKey::plugin("a-b_c9", "x-y_z0").is_ok());
        // 長すぎる区画は弾く。
        assert!(SecretKey::plugin(&"a".repeat(MAX_SEGMENT_LEN), "pat").is_ok());
        assert!(SecretKey::plugin(&"a".repeat(MAX_SEGMENT_LEN + 1), "pat").is_err());
    }

    #[test]
    fn memory_store_round_trips() {
        let store = MemorySecretStore::new();
        let key = SecretKey::mcp_token();
        assert_eq!(store.get(&key).unwrap(), None);

        store.set(&key, "s3cret").unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("s3cret"));
        assert_eq!(store.keys(), vec!["core/mcp-token".to_owned()]);

        store.set(&key, "next").unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("next"));

        store.delete(&key).unwrap();
        assert_eq!(store.get(&key).unwrap(), None);
        // 消し直しても成功扱い。
        store.delete(&key).unwrap();
        assert!(store.keys().is_empty());
    }

    #[test]
    fn put_trims_and_treats_blank_as_unset() {
        let store = MemorySecretStore::new();
        let key = SecretKey::mcp_token();

        assert!(put(&store, &key, Some("  s3cret  ")).unwrap());
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("s3cret"));

        assert!(!put(&store, &key, Some("   ")).unwrap());
        assert_eq!(store.get(&key).unwrap(), None);

        put(&store, &key, Some("again")).unwrap();
        assert!(!put(&store, &key, None).unwrap());
        assert_eq!(store.get(&key).unwrap(), None);
    }

    #[test]
    fn a_broken_store_reports_errors_instead_of_falling_back() {
        let store = MemorySecretStore::failing();
        let key = SecretKey::mcp_token();
        assert!(matches!(
            store.set(&key, "s3cret"),
            Err(SecretError::Backend { .. })
        ));
        assert!(store.get(&key).is_err());
        assert!(store.delete(&key).is_err());
        assert!(put(&store, &key, Some("x")).is_err());
        // 失敗したのだから値は残らない(平文フォールバックは無い)。
        assert!(store.keys().is_empty());
    }

    #[test]
    fn error_messages_are_readable() {
        let message = SecretError::InvalidKey {
            segment: "bad/key".to_owned(),
        }
        .to_string();
        assert!(message.contains("bad/key"), "{message}");

        let message = SecretError::Backend {
            key: "core/mcp-token".to_owned(),
            message: "アクセスが拒否されました".to_owned(),
        }
        .to_string();
        assert!(message.contains("core/mcp-token"), "{message}");
        assert!(message.contains("アクセスが拒否されました"), "{message}");
    }

    /// 実際の資格情報マネージャーを 1 往復する。
    ///
    /// エントリはユーザー単位でグローバルなので、既定では走らせない。
    /// service 名にはテスト専用の接尾辞を付け、最後に必ず消す。
    ///
    /// ```powershell
    /// cargo test -p questloom-desktop --lib secrets -- --ignored
    /// ```
    #[test]
    #[ignore = "OS の資格情報マネージャーに書き込む (--ignored で実行)"]
    fn keyring_store_round_trips_against_the_real_backend() {
        let service = format!("{DEFAULT_SERVICE}-test-{}", std::process::id());
        let store = KeyringSecretStore::new(&service);
        let key = SecretKey::plugin("selftest", "value").unwrap();

        // 後始末を必ず通すため、以降は結果を受け取ってから assert する。
        let before = store.get(&key);
        let written = store.set(&key, "s3cret");
        let read = store.get(&key);
        let removed = store.delete(&key);
        let after = store.get(&key);
        // 消し残しがないよう、失敗しても最後にもう一度消しておく。
        let _ = store.delete(&key);

        assert_eq!(before.expect("未設定を読める"), None);
        written.expect("書ける");
        assert_eq!(read.expect("読める").as_deref(), Some("s3cret"));
        removed.expect("消せる");
        assert_eq!(after.expect("消した後も読める"), None);
    }
}
