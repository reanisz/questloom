//! アプリ状態の初期化。DB を開き、マイグレーション・起動時バックアップを実行する。

use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};

use questloom_core::clock::SystemClock;
use questloom_core::repository::TaskRepository;
use questloom_core::service::TaskService;
use questloom_core::settings::{BoardSettings, CoreSettings, CORE_NAMESPACE};
use questloom_store::{backup, SqliteStore, StoreError};

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
    /// `%APPDATA%\questloom` 相当のデータディレクトリ。
    pub data_dir: PathBuf,
    /// 現在のコア設定。[`save_settings`](Self::save_settings) で差し替える。
    settings: RwLock<CoreSettings>,
}

impl AppState {
    /// データディレクトリを準備し、DB を開いてサービスを構築する。
    ///
    /// 起動時に以下を行う。
    /// 1. ディレクトリ作成
    /// 2. DB オープン(PRAGMA 設定 + マイグレーション)
    /// 3. コア設定の読み込み
    /// 4. 起動時バックアップ(世代数は設定値、既定 14)
    ///
    /// # Errors
    /// ディレクトリ作成またはストア初期化に失敗した場合。
    /// バックアップの失敗は起動を妨げない(警告ログのみ)。
    pub fn initialize(data_dir: &Path) -> Result<Self, SetupError> {
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

        let settings: CoreSettings = store.get_settings(CORE_NAMESPACE)?;
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
            data_dir: data_dir.to_path_buf(),
            settings: RwLock::new(settings),
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

    /// コア設定を永続化し、サービスとデスクトップ側へ反映する。
    ///
    /// 反映の順序は「保存 → 保持している値の差し替え → 通知」。
    /// [`DomainEvent::SettingsChanged`](questloom_core::events::DomainEvent::SettingsChanged)
    /// を受け取った watcher が新しい値を読めるようにするため、通知は最後に行う。
    ///
    /// # Errors
    /// 設定の保存に失敗した場合。
    pub fn save_settings(&self, settings: CoreSettings) -> Result<(), StoreError> {
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
    use questloom_core::service::NewTask;

    #[test]
    fn initialize_creates_db_and_startup_backup() {
        let dir = tempfile::tempdir().expect("一時ディレクトリ");
        let state = AppState::initialize(dir.path()).expect("初期化できる");
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
        let state = AppState::initialize(dir.path()).unwrap();
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
        let state = AppState::initialize(dir.path()).unwrap();
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
        let state = AppState::initialize(dir.path()).unwrap();
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
}
