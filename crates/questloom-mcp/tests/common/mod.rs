//! 統合テスト共通のセットアップ。

use std::sync::Arc;

use questloom_core::clock::SystemClock;
use questloom_core::repository::TaskRepository;
use questloom_core::service::TaskService;
use questloom_core::settings::BoardSettings;
use questloom_store::SqliteStore;

/// インメモリ SQLite に載せた、既定設定のサービスを作る。
pub fn service() -> Arc<TaskService> {
    let store = Arc::new(SqliteStore::open_in_memory().expect("インメモリ DB を開ける"));
    let repository: Arc<dyn TaskRepository> = store as Arc<dyn TaskRepository>;
    Arc::new(TaskService::new(
        repository,
        Arc::new(SystemClock),
        BoardSettings::default(),
    ))
}
