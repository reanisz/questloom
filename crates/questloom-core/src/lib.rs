//! questloom のドメインモデルとサービス層。UI・Tauri・HTTP に依存しない純粋なコア。
//!
//! この crate は以下を提供する。
//!
//! - [`model`][]: タスク・リソース・アップデート履歴などのドメインモデル
//! - [`bucket`][]: `scheduled_*` から表示用の時間バケットを導出する純粋関数群
//! - [`sort_order`][]: ドラッグ&ドロップ並び替え用の fractional indexing
//! - [`repository`][]: 永続化の抽象 ([`repository::TaskRepository`])
//! - [`service`][]: ユースケースを実装する [`service::TaskService`]
//! - [`events`][]: broadcast チャネルで配信されるドメインイベント
//! - [`settings`][]: コア設定モデル

pub mod bucket;
pub mod clock;
pub mod error;
pub mod events;
pub mod model;
pub mod repository;
pub mod service;
pub mod settings;
pub mod sort_order;

pub use bucket::{
    bucket_for, derive_bucket, scheduled_for_bucket, week_key_of, BoardColumn, Bucket,
};
pub use clock::{Clock, FixedClock, SystemClock};
pub use error::{CoreError, CoreResult};
pub use events::DomainEvent;
pub use model::{
    Origin, ResourceId, ResourceKind, Task, TaskId, TaskResource, TaskStatus, TaskUpdateEntry,
    UpdateId, WeekKey,
};
pub use model::{Scheduled, ScheduledKind};
pub use repository::{RepoResult, RepositoryError, TaskRepository};
pub use service::{
    Board, BoardColumns, MoveRequest, NewResource, NewTask, TaskCard, TaskDetail, TaskPatch,
    TaskService,
};
pub use settings::{AiProvider, BoardSettings, CoreSettings, SettingsError, WeekStart};
