//! アプリ状態の初期化。DB を開き、マイグレーション・起動時バックアップを実行する。
//!
//! シークレット(いまのところ内蔵 MCP サーバーの Bearer トークン)は DB ではなく
//! OS の資格情報ストアに置く([`crate::secrets`])。旧バージョンが設定 JSON に
//! 平文で保存していた場合は、起動時に 1 度だけ資格情報ストアへ移送し、JSON からは消す。

use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};

use questloom_core::clock::SystemClock;
use questloom_core::repository::TaskRepository;
use questloom_core::service::TaskService;
use questloom_core::settings::{BoardSettings, CoreSettings, CORE_NAMESPACE};
use questloom_store::{backup, SqliteStore, StoreError};

use crate::secrets::{self, SecretError, SecretKey, SecretStore};

/// DB ファイル名。
const DB_FILE: &str = "data.db";
/// バックアップ格納ディレクトリ名。
const BACKUPS_DIR: &str = "backups";

/// アプリ起動時のセットアップに失敗したことを表す。
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    /// データディレクトリを解決・作成できない。
    #[error("データディレクトリを準備できません ({path}): {source}")]
    DataDir {
        /// 対象パス。
        path: PathBuf,
        /// 元エラー。
        #[source]
        source: std::io::Error,
    },
    /// ストアの初期化に失敗した。
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Tauri の State として保持するアプリ状態。
///
/// [`TaskService`] は内部で排他制御を行うため、ここでは共有参照のみを持つ。
/// コア設定の「いま有効な値」を持つのもここ(サービスはボード表示に要る分だけ
/// を受け取る)。
pub struct AppState {
    /// タスク操作のサービス層。
    pub service: Arc<TaskService>,
    /// 永続化ストア(設定・バックアップに使う)。
    pub store: Arc<SqliteStore>,
    /// シークレットの保存先(OS の資格情報ストア)。
    pub secrets: Arc<dyn SecretStore>,
    /// `%APPDATA%\questloom` 相当のデータディレクトリ。
    pub data_dir: PathBuf,
    /// 現在のコア設定。[`save_settings`](Self::save_settings) で差し替える。
    settings: RwLock<CoreSettings>,
    /// 現在の MCP トークン。実体は資格情報ストアで、ここはその写し。
    ///
    /// 写しを持つのは、資格情報ストアが一時的に読めなくなったときに
    /// 「トークン未設定」と誤認して **MCP サーバーを認証なしで起動してしまう**のを
    /// 避けるため。書き込み([`set_mcp_token`](Self::set_mcp_token))が成功した
    /// ときだけ差し替える。
    mcp_token: RwLock<Option<String>>,
}

impl AppState {
    /// データディレクトリを準備し、DB を開いてサービスを構築する。
    ///
    /// シークレットの保存先は [`secrets::default_store`](crate::secrets::default_store)。
    ///
    /// # Errors
    /// ディレクトリ作成またはストア初期化に失敗した場合。
    pub fn initialize(data_dir: &Path) -> Result<Self, SetupError> {
        Self::initialize_with_secrets(data_dir, secrets::default_store())
    }

    /// シークレットの保存先を指定して初期化する(テスト用)。
    ///
    /// 起動時に以下を行う。
    /// 1. ディレクトリ作成
    /// 2. DB オープン(PRAGMA 設定 + マイグレーション)
    /// 3. コア設定の読み込みと、平文 MCP トークンの資格情報ストアへの移送
    /// 4. 起動時バックアップ(世代数は設定値、既定 14)
    ///
    /// # Errors
    /// ディレクトリ作成またはストア初期化に失敗した場合。
    /// バックアップとシークレットの移送の失敗は起動を妨げない(ログのみ)。
    pub fn initialize_with_secrets(
        data_dir: &Path,
        secrets: Arc<dyn SecretStore>,
    ) -> Result<Self, SetupError> {
        std::fs::create_dir_all(data_dir).map_err(|source| SetupError::DataDir {
            path: data_dir.to_path_buf(),
            source,
        })?;

        let store = Arc::new(SqliteStore::open(data_dir.join(DB_FILE))?);
        tracing::info!(
            path = %store.path().display(),
            version = store.schema_version()?,
            "データベースを開きました"
        );

        let mut settings: CoreSettings = store.get_settings(CORE_NAMESPACE)?;
        let mcp_token = adopt_mcp_token(&store, secrets.as_ref(), &mut settings);
        let backups_dir = data_dir.join(BACKUPS_DIR);
        match backup::create_backup(&store, &backups_dir, settings.backup_generations) {
            Ok(path) => tracing::info!(path = %path.display(), "起動時バックアップを作成しました"),
            // バックアップに失敗してもアプリは使えるべきなので、起動は継続する。
            Err(error) => tracing::error!(%error, "起動時バックアップに失敗しました"),
        }

        let repository: Arc<dyn TaskRepository> = Arc::clone(&store) as Arc<dyn TaskRepository>;
        let service = Arc::new(TaskService::new(
            repository,
            Arc::new(SystemClock),
            BoardSettings::from(&settings),
        ));

        Ok(Self {
            service,
            store,
            secrets,
            data_dir: data_dir.to_path_buf(),
            settings: RwLock::new(settings),
            mcp_token: RwLock::new(mcp_token),
        })
    }

    /// 現在のコア設定を返す。
    #[must_use]
    pub fn settings(&self) -> CoreSettings {
        self.settings
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// 現在の MCP トークン。`None` なら認証なし。
    #[must_use]
    pub fn mcp_token(&self) -> Option<String> {
        self.mcp_token
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// MCP トークンが設定されているか。**値そのものは返さない**。
    ///
    /// 設定画面へはこれだけを渡す(一度書いたトークンは読み出せない)。
    #[must_use]
    pub fn mcp_token_is_set(&self) -> bool {
        self.mcp_token
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    /// MCP トークンを設定・解除する。設定後の状態(設定済みなら `true`)を返す。
    ///
    /// 前後の空白は落とし、空文字列・`None` は「認証なし」として消す。
    /// 資格情報ストアへ書けなかった場合は**平文へ落とさずエラーにする**ので、
    /// 保持している値も変えない。成功したら `SettingsChanged` を発行し、
    /// MCP サーバーを張り直させる。
    ///
    /// # Errors
    /// 資格情報ストアの読み書きに失敗した場合。
    pub fn set_mcp_token(&self, token: Option<&str>) -> Result<bool, SecretError> {
        let key = SecretKey::mcp_token();
        let configured = secrets::put(self.secrets.as_ref(), &key, token)?;
        *self
            .mcp_token
            .write()
            .unwrap_or_else(PoisonError::into_inner) =
            configured.then(|| token.unwrap_or_default().trim().to_owned());
        self.service.notify_settings_changed();
        Ok(configured)
    }

    /// コア設定を永続化し、サービスとデスクトップ側へ反映する。
    ///
    /// 反映の順序は「保存 → 保持している値の差し替え → 通知」。
    /// [`DomainEvent::SettingsChanged`](questloom_core::events::DomainEvent::SettingsChanged)
    /// を受け取った watcher が新しい値を読めるようにするため、通知は最後に行う。
    ///
    /// # Errors
    /// 設定の保存に失敗した場合。
    pub fn save_settings(&self, settings: CoreSettings) -> Result<(), StoreError> {
        // シークレットはここを通らない(MCP トークンは set_mcp_token 専用)。
        // 呼び出し元が旧フィールドに値を入れてきても、保持もしないし書きもしない。
        let settings = CoreSettings {
            legacy_mcp_token: None,
            ..settings
        };
        self.store.set_settings(CORE_NAMESPACE, &settings)?;
        self.service
            .set_board_settings(BoardSettings::from(&settings));
        *self
            .settings
            .write()
            .unwrap_or_else(PoisonError::into_inner) = settings;
        self.service.notify_settings_changed();
        Ok(())
    }
}

/// 起動時に MCP トークンの「いま有効な値」を決める。
///
/// 手順は次のとおりで、`settings.legacy_mcp_token` は必ず `None` にして返す
/// (呼び出し元が保持する設定に平文が残らないようにするため)。
///
/// 1. 資格情報ストアを読む。値があればそれが正。
/// 2. 旧バージョンが設定 JSON に平文で残していたら、資格情報ストアへ移送し、
///    設定 JSON から消す(**1 回限りのマイグレーション**)。
/// 3. 移送に失敗したら error ログを出し、**平文は消さずに残す**。値は返すので
///    サーバーの認証は効いたまま動き、次回起動でもう一度移送を試みる。
fn adopt_mcp_token(
    store: &SqliteStore,
    secrets: &dyn SecretStore,
    settings: &mut CoreSettings,
) -> Option<String> {
    let key = SecretKey::mcp_token();
    let stored = match secrets.get(&key) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(%error, "資格情報ストアから MCP トークンを読めませんでした");
            None
        }
    };

    let legacy = settings
        .legacy_mcp_token
        .take()
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty());

    let Some(legacy) = legacy else {
        return stored;
    };

    // 資格情報ストア側に値があるなら、そちらが正。平文は落とすだけでよい。
    if let Some(stored) = stored {
        tracing::warn!("資格情報ストアに MCP トークンがあるため、設定に残っていた平文を破棄します");
        forget_legacy_mcp_token(store, settings);
        return Some(stored);
    }

    match secrets.set(&key, &legacy) {
        Ok(()) => {
            tracing::info!("平文の MCP トークンを資格情報マネージャーへ移しました");
            forget_legacy_mcp_token(store, settings);
        }
        // 平文へ戻すことはしないが、既にある平文を消しもしない(次回起動でやり直す)。
        Err(error) => tracing::error!(
            %error,
            "MCP トークンを資格情報マネージャーへ移せませんでした。設定に平文のまま残ります"
        ),
    }
    Some(legacy)
}

/// 設定 JSON を書き直して、残っていた平文トークンを消す。
///
/// `CoreSettings` は `mcpToken` を serialize しないので、書き直すだけで消える。
fn forget_legacy_mcp_token(store: &SqliteStore, settings: &CoreSettings) {
    if let Err(error) = store.set_settings(CORE_NAMESPACE, settings) {
        tracing::warn!(%error, "設定から平文の MCP トークンを消せませんでした");
    }
}

/// テスト用の組み立てヘルパ。
///
/// インメモリ DB に載せたサービスは複数のモジュールのテストで要るため、
/// ここにまとめる。
#[cfg(test)]
pub mod test_support {
    use super::{Arc, SystemClock, TaskRepository, TaskService};
    use questloom_core::settings::BoardSettings;
    use questloom_store::SqliteStore;

    /// インメモリ SQLite に載せたサービスを作る。
    pub fn service(board: BoardSettings) -> Arc<TaskService> {
        let store = Arc::new(SqliteStore::open_in_memory().expect("インメモリ DB"));
        let repository: Arc<dyn TaskRepository> = store as Arc<dyn TaskRepository>;
        Arc::new(TaskService::new(repository, Arc::new(SystemClock), board))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemorySecretStore;
    use questloom_core::service::NewTask;

    /// 資格情報マネージャーはユーザー単位でグローバルなので、単体テストは
    /// **必ず**インメモリのストアで初期化する(`AppState::initialize` は使わない)。
    fn initialize(dir: &Path) -> Result<AppState, SetupError> {
        AppState::initialize_with_secrets(dir, Arc::new(MemorySecretStore::new()))
    }

    #[test]
    fn initialize_creates_db_and_startup_backup() {
        let dir = tempfile::tempdir().expect("一時ディレクトリ");
        let state = initialize(dir.path()).expect("初期化できる");
        assert!(dir.path().join(DB_FILE).exists());
        assert_eq!(
            backup::list_backups(&dir.path().join(BACKUPS_DIR))
                .unwrap()
                .len(),
            1
        );

        state
            .service
            .create_task(NewTask {
                title: "起動確認".to_owned(),
                ..NewTask::default()
            })
            .unwrap();
        assert_eq!(state.service.board().unwrap().columns.new.len(), 1);
    }

    #[test]
    fn settings_are_saved_and_applied() {
        use questloom_core::settings::WeekStart;

        let dir = tempfile::tempdir().unwrap();
        let state = initialize(dir.path()).unwrap();
        assert_eq!(state.settings().week_start, WeekStart::Monday);
        assert_eq!(state.service.week_start(), WeekStart::Monday);

        state
            .save_settings(CoreSettings {
                week_start: WeekStart::Sunday,
                backup_generations: 3,
                overlay_enabled: false,
                ..CoreSettings::default()
            })
            .unwrap();
        assert_eq!(state.settings().week_start, WeekStart::Sunday);
        // ボード表示に関わる分はサービスにも渡る。
        assert_eq!(state.service.week_start(), WeekStart::Sunday);

        // 再起動しても保持される。
        drop(state);
        let state = initialize(dir.path()).unwrap();
        assert_eq!(state.settings().week_start, WeekStart::Sunday);
        assert_eq!(state.service.week_start(), WeekStart::Sunday);
        assert_eq!(state.settings().backup_generations, 3);
        assert!(!state.settings().overlay_enabled);
        // 保存していない項目は既定値のまま残る。
        assert_eq!(state.settings().global_shortcut, "Ctrl+Space");
    }

    #[tokio::test]
    async fn saving_settings_notifies_subscribers_after_the_new_value_is_visible() {
        let dir = tempfile::tempdir().unwrap();
        let state = initialize(dir.path()).unwrap();
        let mut events = state.service.subscribe();

        state
            .save_settings(CoreSettings {
                overlay_enabled: false,
                ..CoreSettings::default()
            })
            .unwrap();

        assert_eq!(
            events.recv().await.unwrap(),
            questloom_core::events::DomainEvent::SettingsChanged
        );
        assert!(!state.settings().overlay_enabled);
    }

    // ---- MCP トークン(資格情報ストア) ----

    /// 設定 JSON に平文の `mcpToken` を仕込む(旧バージョンが保存した状態の再現)。
    fn seed_plaintext_token(dir: &Path, token: &str) {
        let store = SqliteStore::open(dir.join(DB_FILE)).expect("DB を開ける");
        let json = serde_json::json!({ "mcpPort": 39150, "mcpToken": token });
        store
            .set_settings(CORE_NAMESPACE, &json)
            .expect("平文の設定を書ける");
    }

    /// 設定 JSON の生文字列。移送後に平文が残っていないことの確認に使う。
    fn raw_settings(dir: &Path) -> String {
        let store = SqliteStore::open(dir.join(DB_FILE)).expect("DB を開ける");
        store
            .get_settings_json(CORE_NAMESPACE)
            .expect("読める")
            .unwrap_or_default()
    }

    #[test]
    fn a_fresh_profile_has_no_token() {
        let dir = tempfile::tempdir().unwrap();
        let state = initialize(dir.path()).unwrap();
        assert_eq!(state.mcp_token(), None);
        assert!(!state.mcp_token_is_set());
    }

    #[test]
    fn setting_the_token_writes_only_to_the_secret_store() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecretStore::new());
        let state = AppState::initialize_with_secrets(
            dir.path(),
            Arc::clone(&secrets) as Arc<dyn SecretStore>,
        )
        .unwrap();

        assert!(state.set_mcp_token(Some("  s3cret  ")).unwrap());
        assert_eq!(
            state.mcp_token().as_deref(),
            Some("s3cret"),
            "前後の空白は落とす"
        );
        assert!(state.mcp_token_is_set());
        assert_eq!(
            secrets.get(&SecretKey::mcp_token()).unwrap().as_deref(),
            Some("s3cret")
        );
        assert_eq!(secrets.keys(), vec!["core/mcp-token".to_owned()]);

        // 設定を保存しても、DB にトークンは入らない。
        state.save_settings(CoreSettings::default()).unwrap();
        assert!(!raw_settings(dir.path()).contains("s3cret"));

        // 空白のみ・None は「認証なし」。
        assert!(!state.set_mcp_token(Some("   ")).unwrap());
        assert_eq!(state.mcp_token(), None);
        assert!(secrets.keys().is_empty());
        assert!(!state.set_mcp_token(None).unwrap());
    }

    #[test]
    fn the_token_survives_a_restart_through_the_secret_store() {
        let dir = tempfile::tempdir().unwrap();
        let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());

        let state = AppState::initialize_with_secrets(dir.path(), Arc::clone(&secrets)).unwrap();
        state.set_mcp_token(Some("s3cret")).unwrap();
        drop(state);

        let state = AppState::initialize_with_secrets(dir.path(), secrets).unwrap();
        assert_eq!(state.mcp_token().as_deref(), Some("s3cret"));
    }

    /// 既存ユーザーの移行パス: 平文 → 資格情報ストア、設定 JSON からは消える。
    #[test]
    fn a_plaintext_token_is_migrated_into_the_secret_store_once() {
        let dir = tempfile::tempdir().unwrap();
        seed_plaintext_token(dir.path(), "  legacy-token  ");
        assert!(
            raw_settings(dir.path()).contains("legacy-token"),
            "仕込みの確認"
        );

        let secrets = Arc::new(MemorySecretStore::new());
        let state = AppState::initialize_with_secrets(
            dir.path(),
            Arc::clone(&secrets) as Arc<dyn SecretStore>,
        )
        .unwrap();

        assert_eq!(state.mcp_token().as_deref(), Some("legacy-token"));
        assert_eq!(
            secrets.get(&SecretKey::mcp_token()).unwrap().as_deref(),
            Some("legacy-token")
        );
        // 平文は設定からも、保持している設定値からも消えている。
        assert!(!raw_settings(dir.path()).contains("legacy-token"));
        assert_eq!(state.settings().legacy_mcp_token, None);
        // 他の設定は失われない。
        assert_eq!(state.settings().mcp_port, 39150);

        // 2 回目の起動では移送するものが無い(冪等)。
        drop(state);
        let state = AppState::initialize_with_secrets(
            dir.path(),
            Arc::clone(&secrets) as Arc<dyn SecretStore>,
        )
        .unwrap();
        assert_eq!(state.mcp_token().as_deref(), Some("legacy-token"));
        assert_eq!(secrets.keys(), vec!["core/mcp-token".to_owned()]);
    }

    /// 資格情報ストア側に値があるなら、そちらが正。平文は捨てるだけ。
    #[test]
    fn the_secret_store_wins_over_a_leftover_plaintext_token() {
        let dir = tempfile::tempdir().unwrap();
        seed_plaintext_token(dir.path(), "legacy-token");

        let secrets = Arc::new(MemorySecretStore::new());
        secrets
            .set(&SecretKey::mcp_token(), "current-token")
            .unwrap();

        let state = AppState::initialize_with_secrets(
            dir.path(),
            Arc::clone(&secrets) as Arc<dyn SecretStore>,
        )
        .unwrap();
        assert_eq!(state.mcp_token().as_deref(), Some("current-token"));
        assert!(!raw_settings(dir.path()).contains("legacy-token"));
    }

    /// 資格情報ストアが使えないとき、平文へフォールバックしない。
    ///
    /// 保存は失敗し(= UI にエラーが出る)、既存の平文は消さずに残して
    /// 次回起動でやり直す。認証は効いたまま(トークンは返る)。
    #[test]
    fn a_broken_secret_store_fails_loudly_and_keeps_the_plaintext_for_a_retry() {
        let dir = tempfile::tempdir().unwrap();
        seed_plaintext_token(dir.path(), "legacy-token");

        let state = AppState::initialize_with_secrets(
            dir.path(),
            Arc::new(MemorySecretStore::failing()) as Arc<dyn SecretStore>,
        )
        .unwrap();

        // 移送できなくても、動いているサーバーの認証は落とさない。
        assert_eq!(state.mcp_token().as_deref(), Some("legacy-token"));
        // 次回起動でやり直せるよう、平文は残したまま。
        assert!(raw_settings(dir.path()).contains("legacy-token"));

        // 新しい値の保存はエラーになり、保持している値も変わらない。
        assert!(state.set_mcp_token(Some("next")).is_err());
        assert_eq!(state.mcp_token().as_deref(), Some("legacy-token"));
    }

    /// 空文字列の平文トークンは「未設定」。移送もしない。
    #[test]
    fn a_blank_plaintext_token_is_treated_as_unset() {
        let dir = tempfile::tempdir().unwrap();
        seed_plaintext_token(dir.path(), "   ");

        let secrets = Arc::new(MemorySecretStore::new());
        let state = AppState::initialize_with_secrets(
            dir.path(),
            Arc::clone(&secrets) as Arc<dyn SecretStore>,
        )
        .unwrap();
        assert_eq!(state.mcp_token(), None);
        assert!(secrets.keys().is_empty());
    }

    #[tokio::test]
    async fn changing_the_token_notifies_subscribers_so_the_server_is_rebuilt() {
        let dir = tempfile::tempdir().unwrap();
        let state = initialize(dir.path()).unwrap();
        let mut events = state.service.subscribe();

        state.set_mcp_token(Some("s3cret")).unwrap();
        assert_eq!(
            events.recv().await.unwrap(),
            questloom_core::events::DomainEvent::SettingsChanged
        );
    }
}
