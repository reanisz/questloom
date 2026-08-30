//! ユースケースを実装するサービス層。
//!
//! [`TaskService`] は [`TaskRepository`] を介して永続化し、操作に応じて
//! [`DomainEvent`] を broadcast チャネルへ発行する。
//! すべてのメソッドは `&self` を取り、内部で排他制御を行う。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::bucket::{derive_bucket, BoardColumn, Bucket};
use crate::clock::Clock;
use crate::error::{CoreError, CoreResult};
use crate::events::DomainEvent;
use crate::model::{
    Origin, ResourceId, ResourceKind, Scheduled, Task, TaskId, TaskResource, TaskStatus,
    TaskUpdateEntry, UpdateId,
};
use crate::repository::TaskRepository;
use crate::settings::{CoreSettings, WeekStart};
use crate::sort_order;

/// ドメインイベントのブロードキャスト容量。
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// タスク作成の入力。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NewTask {
    /// タイトル(必須。空白のみは不可)。
    pub title: String,
    /// 詳細。
    pub description: String,
    /// 状態。既定は `New`。
    pub status: Option<TaskStatus>,
    /// 予定。
    pub scheduled: Scheduled,
    /// 締切。
    pub deadline: Option<DateTime<Utc>>,
    /// インスタントタスクか。
    pub is_instant: bool,
    /// 発生元。既定は `User`。
    pub origin: Origin,
    /// 親タスク。
    pub parent_id: Option<TaskId>,
    /// 同時に登録する関連リソース。
    pub resources: Vec<NewResource>,
}

/// タスク更新の入力。`None` のフィールドは変更しない。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TaskPatch {
    /// タイトル。
    pub title: Option<String>,
    /// 詳細。
    pub description: Option<String>,
    /// 締切を設定する。
    pub deadline: Option<DateTime<Utc>>,
    /// 締切を消す(`deadline` より優先)。
    pub clear_deadline: bool,
    /// 予定。
    pub scheduled: Option<Scheduled>,
    /// インスタントフラグ。
    pub is_instant: Option<bool>,
}

/// タスク移動の入力。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveRequest {
    /// 移動先の列。
    pub column: BoardColumn,
    /// 移動先で直前に来るタスク。
    #[serde(default)]
    pub prev_id: Option<TaskId>,
    /// 移動先で直後に来るタスク。
    #[serde(default)]
    pub next_id: Option<TaskId>,
}

impl MoveRequest {
    /// 列の末尾へ移動するリクエストを作る。
    #[must_use]
    pub const fn to_column(column: BoardColumn) -> Self {
        Self {
            column,
            prev_id: None,
            next_id: None,
        }
    }
}

/// 関連リソース追加の入力。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewResource {
    /// 種別。
    pub kind: ResourceKind,
    /// URL またはファイルパス。
    pub value: String,
    /// 表示ラベル。
    #[serde(default)]
    pub label: String,
    /// 主リソースにするか。
    #[serde(default)]
    pub is_primary: bool,
}

/// ボード表示用のタスク 1 件。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskCard {
    /// タスク本体(JSON では平坦化される)。
    #[serde(flatten)]
    pub task: Task,
    /// 導出された時間バケット(`status == Todo` のときのみ)。
    pub bucket: Option<Bucket>,
    /// 子タスク数。
    pub child_count: usize,
    /// 関連リソース数。
    pub resource_count: usize,
    /// 主リソース(オーバーレイのワンクリック起動対象)。
    pub primary_resource: Option<TaskResource>,
}

/// ボードの各列。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardColumns {
    /// New 列。
    pub new: Vec<TaskCard>,
    /// Today 列。
    pub today: Vec<TaskCard>,
    /// Tomorrow 列。
    pub tomorrow: Vec<TaskCard>,
    /// This Week 列。
    pub this_week: Vec<TaskCard>,
    /// Next Week 列。
    pub next_week: Vec<TaskCard>,
    /// Future 列。
    pub future: Vec<TaskCard>,
    /// Doing 列。
    pub doing: Vec<TaskCard>,
    /// Done 列。
    pub done: Vec<TaskCard>,
}

/// ボード全体。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    /// バケット導出に使った「今日」。
    pub today: NaiveDate,
    /// バケット導出に使った週開始曜日。
    pub week_start: WeekStart,
    /// 各列のタスク(いずれも `sort_order` 昇順)。
    pub columns: BoardColumns,
}

/// タスク詳細。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDetail {
    /// タスク本体(JSON では平坦化される)。
    #[serde(flatten)]
    pub card: TaskCard,
    /// 関連リソース。
    pub resources: Vec<TaskResource>,
    /// アップデート履歴(古い順)。
    pub updates: Vec<TaskUpdateEntry>,
    /// 親タスク。
    pub parent: Option<TaskCard>,
    /// 子タスク。
    pub children: Vec<TaskCard>,
}

/// タスク関連のユースケース。
pub struct TaskService {
    repo: Arc<dyn TaskRepository>,
    clock: Arc<dyn Clock>,
    settings: RwLock<CoreSettings>,
    /// 「読んで書く」操作を直列化するためのロック。
    write_lock: Mutex<()>,
    events: broadcast::Sender<DomainEvent>,
}

impl std::fmt::Debug for TaskService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskService")
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

impl TaskService {
    /// サービスを構築する。
    #[must_use]
    pub fn new(
        repo: Arc<dyn TaskRepository>,
        clock: Arc<dyn Clock>,
        settings: CoreSettings,
    ) -> Self {
        let (events, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            repo,
            clock,
            settings: RwLock::new(settings),
            write_lock: Mutex::new(()),
            events,
        }
    }

    /// ドメインイベントを購読する。
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<DomainEvent> {
        self.events.subscribe()
    }

    /// 現在のコア設定を返す。
    #[must_use]
    pub fn settings(&self) -> CoreSettings {
        self.read_settings()
    }

    /// コア設定を差し替え、[`DomainEvent::SettingsChanged`] を発行する。
    pub fn set_settings(&self, settings: CoreSettings) {
        if let Ok(mut guard) = self.settings.write() {
            *guard = settings;
        }
        self.emit(DomainEvent::SettingsChanged);
    }

    /// 現在のクロックにおける「今日」。
    #[must_use]
    pub fn today(&self) -> NaiveDate {
        self.clock.today()
    }

    /// 日付が変わったことを通知する(1 分毎の監視から呼ぶ)。
    pub fn notify_day_changed(&self, date: NaiveDate) {
        self.emit(DomainEvent::DayChanged { date });
    }

    // ---- 参照系 ----

    /// ボード全体を、バケット導出済みの構造で返す。
    ///
    /// # Errors
    /// 永続化層のエラー。
    pub fn board(&self) -> CoreResult<Board> {
        let today = self.clock.today();
        let week_start = self.read_settings().week_start;

        let tasks = self.repo.list_tasks()?;
        let resources = self.repo.list_all_resources()?;

        let mut child_counts: HashMap<TaskId, usize> = HashMap::new();
        for task in &tasks {
            if let Some(parent) = task.parent_id {
                *child_counts.entry(parent).or_insert(0) += 1;
            }
        }
        let mut resource_index: HashMap<TaskId, (usize, Option<TaskResource>)> = HashMap::new();
        for resource in resources {
            let entry = resource_index.entry(resource.task_id).or_insert((0, None));
            entry.0 += 1;
            if resource.is_primary && entry.1.is_none() {
                entry.1 = Some(resource);
            }
        }

        let mut columns = BoardColumns::default();
        for task in tasks {
            let card = build_card(task, today, week_start, &child_counts, &resource_index);
            let target = match card.task.status {
                TaskStatus::New => &mut columns.new,
                TaskStatus::Doing => &mut columns.doing,
                TaskStatus::Done => &mut columns.done,
                TaskStatus::Todo => match card.bucket.unwrap_or(Bucket::Future) {
                    Bucket::Today => &mut columns.today,
                    Bucket::Tomorrow => &mut columns.tomorrow,
                    Bucket::ThisWeek => &mut columns.this_week,
                    Bucket::NextWeek => &mut columns.next_week,
                    Bucket::Future => &mut columns.future,
                },
            };
            target.push(card);
        }

        Ok(Board {
            today,
            week_start,
            columns,
        })
    }

    /// タスク詳細(リソース・履歴・親子込み)を返す。
    ///
    /// # Errors
    /// タスクが存在しない場合、または永続化層のエラー。
    pub fn task_detail(&self, id: TaskId) -> CoreResult<TaskDetail> {
        let today = self.clock.today();
        let week_start = self.read_settings().week_start;
        let task = self.require_task(id)?;

        let resources = self.repo.list_resources(id)?;
        let updates = self.repo.list_updates(id)?;
        let children = self.repo.list_children(id)?;
        let parent = match task.parent_id {
            Some(parent_id) => self.repo.find_task(parent_id)?,
            None => None,
        };

        let child_counts = HashMap::from([(id, children.len())]);
        let resource_index = HashMap::from([(
            id,
            (
                resources.len(),
                resources.iter().find(|r| r.is_primary).cloned(),
            ),
        )]);

        let card = build_card(task, today, week_start, &child_counts, &resource_index);
        let empty_counts = HashMap::new();
        let empty_resources = HashMap::new();
        Ok(TaskDetail {
            card,
            resources,
            updates,
            parent: parent.map(|parent| {
                build_card(parent, today, week_start, &empty_counts, &empty_resources)
            }),
            children: children
                .into_iter()
                .map(|child| build_card(child, today, week_start, &empty_counts, &empty_resources))
                .collect(),
        })
    }

    /// タスクを 1 件取得する。
    ///
    /// # Errors
    /// 永続化層のエラー。
    pub fn find_task(&self, id: TaskId) -> CoreResult<Option<Task>> {
        Ok(self.repo.find_task(id)?)
    }

    /// 指定ステータスのタスクを返す。
    ///
    /// # Errors
    /// 永続化層のエラー。
    pub fn list_by_status(&self, status: TaskStatus) -> CoreResult<Vec<Task>> {
        Ok(self.repo.list_tasks_by_status(status)?)
    }

    // ---- 更新系 ----

    /// タスクを作成する。
    ///
    /// # Errors
    /// タイトルが空、親タスクが存在しない、または永続化層のエラー。
    pub fn create_task(&self, input: NewTask) -> CoreResult<Task> {
        let _guard = self.lock_writes();
        let title = normalize_title(&input.title)?;
        let status = input.status.unwrap_or(TaskStatus::New);
        if let Some(parent_id) = input.parent_id {
            self.require_task(parent_id)?;
        }

        let now = self.clock.now();
        let task = Task {
            id: TaskId::new(),
            title,
            description: input.description,
            status,
            scheduled: input.scheduled,
            deadline: input.deadline,
            is_instant: input.is_instant,
            origin: input.origin,
            parent_id: input.parent_id,
            sort_order: self.end_key(status)?,
            created_at: now,
            updated_at: now,
            done_at: if status == TaskStatus::Done {
                Some(now)
            } else {
                None
            },
        };
        self.repo.insert_task(&task)?;

        // 主リソースはちょうど 1 つ。明示指定が無ければ先頭を主リソースにする。
        let primary_index = input
            .resources
            .iter()
            .position(|resource| resource.is_primary)
            .unwrap_or(0);
        let mut previous_key: Option<String> = None;
        for (index, resource) in input.resources.into_iter().enumerate() {
            let key = sort_order::generate_key_between(previous_key.as_deref(), None)?;
            let is_primary = index == primary_index;
            self.repo.insert_resource(&TaskResource {
                id: ResourceId::new(),
                task_id: task.id,
                kind: resource.kind,
                value: resource.value,
                label: resource.label,
                is_primary,
                sort_order: key.clone(),
                created_at: now,
            })?;
            previous_key = Some(key);
        }

        self.emit(DomainEvent::TaskCreated { task_id: task.id });
        Ok(task)
    }

    /// タスクの内容を更新する。
    ///
    /// # Errors
    /// タスクが存在しない、タイトルが空、または永続化層のエラー。
    pub fn update_task(&self, id: TaskId, patch: TaskPatch) -> CoreResult<Task> {
        let _guard = self.lock_writes();
        let mut task = self.require_task(id)?;

        if let Some(title) = patch.title {
            task.title = normalize_title(&title)?;
        }
        if let Some(description) = patch.description {
            task.description = description;
        }
        if patch.clear_deadline {
            task.deadline = None;
        } else if let Some(deadline) = patch.deadline {
            task.deadline = Some(deadline);
        }
        if let Some(scheduled) = patch.scheduled {
            task.scheduled = scheduled;
        }
        if let Some(is_instant) = patch.is_instant {
            task.is_instant = is_instant;
        }
        task.updated_at = self.clock.now();

        self.persist(&task)?;
        self.emit(DomainEvent::TaskUpdated { task_id: id });
        Ok(task)
    }

    /// タスクの状態・予定・並び順を変更する。
    ///
    /// # Errors
    /// タスクが存在しない、並び順キーの生成に失敗、または永続化層のエラー。
    pub fn move_task(&self, id: TaskId, request: MoveRequest) -> CoreResult<Task> {
        let _guard = self.lock_writes();
        let mut task = self.require_task(id)?;
        let today = self.clock.today();
        let week_start = self.read_settings().week_start;

        let (status, scheduled) = request.column.resolve(task.scheduled, today, week_start);
        let was_done = task.status == TaskStatus::Done;

        task.status = status;
        task.scheduled = scheduled;
        task.sort_order = self.sort_key_for(id, status, request.prev_id, request.next_id)?;
        task.updated_at = self.clock.now();
        if status == TaskStatus::Done {
            if !was_done {
                task.done_at = Some(task.updated_at);
            }
        } else {
            task.done_at = None;
        }

        self.persist(&task)?;
        let bucket = bucket_for(&task, today, week_start);
        self.emit(DomainEvent::TaskMoved {
            task_id: id,
            status,
            bucket,
        });
        if status == TaskStatus::Done && !was_done {
            self.emit(DomainEvent::TaskCompleted { task_id: id });
        }
        Ok(task)
    }

    /// タスクを完了にする。
    ///
    /// # Errors
    /// タスクが存在しない、または永続化層のエラー。
    pub fn complete_task(&self, id: TaskId) -> CoreResult<Task> {
        let _guard = self.lock_writes();
        let mut task = self.require_task(id)?;
        if task.status == TaskStatus::Done {
            return Ok(task);
        }
        task.status = TaskStatus::Done;
        task.updated_at = self.clock.now();
        task.done_at = Some(task.updated_at);
        task.sort_order = self.end_key(TaskStatus::Done)?;

        self.persist(&task)?;
        self.emit(DomainEvent::TaskMoved {
            task_id: id,
            status: TaskStatus::Done,
            bucket: None,
        });
        self.emit(DomainEvent::TaskCompleted { task_id: id });
        Ok(task)
    }

    /// インスタントタスクを通常タスクへ昇格する。
    ///
    /// `column` を省略すると Today 列(= `date(today)`)へ置く。
    ///
    /// # Errors
    /// タスクが存在しない、インスタントタスクでない、または永続化層のエラー。
    pub fn promote_task(&self, id: TaskId, column: Option<BoardColumn>) -> CoreResult<Task> {
        let _guard = self.lock_writes();
        let mut task = self.require_task(id)?;
        if !task.is_instant {
            return Err(CoreError::NotInstant(id));
        }
        let today = self.clock.today();
        let week_start = self.read_settings().week_start;
        let column = column.unwrap_or(BoardColumn::Today);
        let (status, scheduled) = column.resolve(task.scheduled, today, week_start);

        task.is_instant = false;
        task.status = status;
        task.scheduled = scheduled;
        task.sort_order = self.end_key(status)?;
        task.updated_at = self.clock.now();
        if status == TaskStatus::Done {
            task.done_at = Some(task.updated_at);
        } else {
            task.done_at = None;
        }

        self.persist(&task)?;
        self.emit(DomainEvent::TaskPromoted { task_id: id });
        self.emit(DomainEvent::TaskMoved {
            task_id: id,
            status,
            bucket: bucket_for(&task, today, week_start),
        });
        Ok(task)
    }

    /// アップデート履歴を追記する。
    ///
    /// # Errors
    /// タスクが存在しない、または永続化層のエラー。
    pub fn add_task_update(
        &self,
        id: TaskId,
        body: impl Into<String>,
        origin: Origin,
    ) -> CoreResult<TaskUpdateEntry> {
        let _guard = self.lock_writes();
        let mut task = self.require_task(id)?;
        let now = self.clock.now();
        let entry = TaskUpdateEntry {
            id: UpdateId::new(),
            task_id: id,
            body: body.into(),
            origin,
            created_at: now,
        };
        self.repo.insert_update(&entry)?;
        task.updated_at = now;
        self.persist(&task)?;
        self.emit(DomainEvent::TaskUpdateAdded { task_id: id });
        Ok(entry)
    }

    /// 関連リソースを追加する。
    ///
    /// 主リソースは 1 タスクに 1 つ。`is_primary` が真なら既存の主リソースを解除する。
    /// 最初のリソースは自動的に主リソースになる。
    ///
    /// # Errors
    /// タスクが存在しない、または永続化層のエラー。
    pub fn add_resource(&self, id: TaskId, input: NewResource) -> CoreResult<TaskResource> {
        let _guard = self.lock_writes();
        let mut task = self.require_task(id)?;
        let existing = self.repo.list_resources(id)?;
        let now = self.clock.now();
        let is_primary = input.is_primary || existing.is_empty();

        if is_primary {
            for mut resource in existing.iter().filter(|r| r.is_primary).cloned() {
                resource.is_primary = false;
                self.repo.update_resource(&resource)?;
            }
        }

        let key =
            sort_order::generate_key_between(existing.last().map(|r| r.sort_order.as_str()), None)?;
        let resource = TaskResource {
            id: ResourceId::new(),
            task_id: id,
            kind: input.kind,
            value: input.value,
            label: input.label,
            is_primary,
            sort_order: key,
            created_at: now,
        };
        self.repo.insert_resource(&resource)?;
        task.updated_at = now;
        self.persist(&task)?;
        self.emit(DomainEvent::TaskResourcesChanged { task_id: id });
        Ok(resource)
    }

    /// 関連リソースを削除する。
    ///
    /// # Errors
    /// タスク・リソースが存在しない、リソースが別タスクに属する、または永続化層のエラー。
    pub fn remove_resource(&self, id: TaskId, resource_id: ResourceId) -> CoreResult<()> {
        let _guard = self.lock_writes();
        let mut task = self.require_task(id)?;
        let existing = self.repo.list_resources(id)?;
        if !existing.iter().any(|r| r.id == resource_id) {
            return Err(CoreError::ResourceNotFound(resource_id));
        }
        self.repo.delete_resource(resource_id)?;
        task.updated_at = self.clock.now();
        self.persist(&task)?;
        self.emit(DomainEvent::TaskResourcesChanged { task_id: id });
        Ok(())
    }

    /// 親タスクを設定・解除する。循環は禁止。
    ///
    /// # Errors
    /// タスクが存在しない、自分自身または子孫を親にしようとした、
    /// または永続化層のエラー。
    pub fn set_parent(&self, id: TaskId, parent_id: Option<TaskId>) -> CoreResult<Task> {
        let _guard = self.lock_writes();
        let mut task = self.require_task(id)?;

        if let Some(parent_id) = parent_id {
            if parent_id == id {
                return Err(CoreError::SelfParent(id));
            }
            self.require_task(parent_id)?;
            // 新しい親から根に向かって辿り、自分に到達したら循環。
            let mut seen = HashSet::new();
            let mut cursor = Some(parent_id);
            while let Some(current) = cursor {
                if current == id {
                    return Err(CoreError::ParentCycle {
                        child: id,
                        parent: parent_id,
                    });
                }
                if !seen.insert(current) {
                    // 既存データが壊れている場合の保険(無限ループ回避)。
                    break;
                }
                cursor = self.repo.find_task(current)?.and_then(|t| t.parent_id);
            }
        }

        task.parent_id = parent_id;
        task.updated_at = self.clock.now();
        self.persist(&task)?;
        self.emit(DomainEvent::TaskParentChanged {
            task_id: id,
            parent_id,
        });
        Ok(task)
    }

    // ---- 内部ヘルパ ----

    fn lock_writes(&self) -> std::sync::MutexGuard<'_, ()> {
        // 毒された場合も処理を継続する(この Mutex は `()` を守るだけで状態を持たない)。
        self.write_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn read_settings(&self) -> CoreSettings {
        self.settings
            .read()
            .map_or_else(|err| err.into_inner().clone(), |guard| guard.clone())
    }

    fn emit(&self, event: DomainEvent) {
        // 購読者がいない場合のエラーは無視してよい。
        let _ = self.events.send(event);
    }

    fn require_task(&self, id: TaskId) -> CoreResult<Task> {
        self.repo
            .find_task(id)?
            .ok_or_else(|| CoreError::TaskNotFound(id))
    }

    fn persist(&self, task: &Task) -> CoreResult<()> {
        if self.repo.update_task(task)? {
            Ok(())
        } else {
            Err(CoreError::TaskNotFound(task.id))
        }
    }

    /// 指定ステータスのリストの末尾に置くための並び順キー。
    fn end_key(&self, status: TaskStatus) -> CoreResult<String> {
        let tasks = self.repo.list_tasks_by_status(status)?;
        let last = tasks.last().map(|task| task.sort_order.clone());
        Ok(sort_order::generate_key_between(last.as_deref(), None)?)
    }

    /// `prev` と `next` の間に入る並び順キー。両方省略なら列の末尾。
    fn sort_key_for(
        &self,
        moving: TaskId,
        status: TaskStatus,
        prev_id: Option<TaskId>,
        next_id: Option<TaskId>,
    ) -> CoreResult<String> {
        let key_of = |id: Option<TaskId>| -> CoreResult<Option<String>> {
            match id.filter(|id| *id != moving) {
                Some(id) => Ok(Some(self.require_task(id)?.sort_order)),
                None => Ok(None),
            }
        };
        let prev = key_of(prev_id)?;
        let next = key_of(next_id)?;
        match (prev, next) {
            (None, None) => self.end_key(status),
            (prev, next) => Ok(sort_order::generate_key_between(
                prev.as_deref(),
                next.as_deref(),
            )?),
        }
    }
}

fn normalize_title(title: &str) -> CoreResult<String> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(CoreError::EmptyTitle);
    }
    Ok(trimmed.to_owned())
}

fn bucket_for(task: &Task, today: NaiveDate, week_start: WeekStart) -> Option<Bucket> {
    (task.status == TaskStatus::Todo).then(|| derive_bucket(&task.scheduled, today, week_start))
}

fn build_card(
    task: Task,
    today: NaiveDate,
    week_start: WeekStart,
    child_counts: &HashMap<TaskId, usize>,
    resource_index: &HashMap<TaskId, (usize, Option<TaskResource>)>,
) -> TaskCard {
    let bucket = bucket_for(&task, today, week_start);
    let child_count = child_counts.get(&task.id).copied().unwrap_or(0);
    let (resource_count, primary_resource) =
        resource_index.get(&task.id).cloned().unwrap_or((0, None));
    TaskCard {
        task,
        bucket,
        child_count,
        resource_count,
        primary_resource,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FixedClock;
    use crate::repository::{RepoResult, RepositoryError};
    use std::sync::Mutex as StdMutex;

    /// テスト用のインメモリリポジトリ。
    #[derive(Default)]
    struct MemoryRepository {
        tasks: StdMutex<Vec<Task>>,
        resources: StdMutex<Vec<TaskResource>>,
        updates: StdMutex<Vec<TaskUpdateEntry>>,
    }

    impl MemoryRepository {
        fn lock<T>(guard: &StdMutex<T>) -> std::sync::MutexGuard<'_, T> {
            guard.lock().unwrap_or_else(|e| e.into_inner())
        }
    }

    impl TaskRepository for MemoryRepository {
        fn insert_task(&self, task: &Task) -> RepoResult<()> {
            let mut tasks = Self::lock(&self.tasks);
            if tasks.iter().any(|t| t.id == task.id) {
                return Err(RepositoryError::message("重複した ID"));
            }
            tasks.push(task.clone());
            Ok(())
        }

        fn update_task(&self, task: &Task) -> RepoResult<bool> {
            let mut tasks = Self::lock(&self.tasks);
            match tasks.iter_mut().find(|t| t.id == task.id) {
                Some(slot) => {
                    *slot = task.clone();
                    Ok(true)
                }
                None => Ok(false),
            }
        }

        fn find_task(&self, id: TaskId) -> RepoResult<Option<Task>> {
            Ok(Self::lock(&self.tasks).iter().find(|t| t.id == id).cloned())
        }

        fn list_tasks(&self) -> RepoResult<Vec<Task>> {
            let mut tasks = Self::lock(&self.tasks).clone();
            tasks.sort_by(|a, b| a.sort_order.cmp(&b.sort_order));
            Ok(tasks)
        }

        fn list_tasks_by_status(&self, status: TaskStatus) -> RepoResult<Vec<Task>> {
            let mut tasks: Vec<Task> = Self::lock(&self.tasks)
                .iter()
                .filter(|t| t.status == status)
                .cloned()
                .collect();
            tasks.sort_by(|a, b| a.sort_order.cmp(&b.sort_order));
            Ok(tasks)
        }

        fn list_children(&self, parent_id: TaskId) -> RepoResult<Vec<Task>> {
            let mut tasks: Vec<Task> = Self::lock(&self.tasks)
                .iter()
                .filter(|t| t.parent_id == Some(parent_id))
                .cloned()
                .collect();
            tasks.sort_by(|a, b| a.sort_order.cmp(&b.sort_order));
            Ok(tasks)
        }

        fn insert_resource(&self, resource: &TaskResource) -> RepoResult<()> {
            Self::lock(&self.resources).push(resource.clone());
            Ok(())
        }

        fn update_resource(&self, resource: &TaskResource) -> RepoResult<bool> {
            let mut resources = Self::lock(&self.resources);
            match resources.iter_mut().find(|r| r.id == resource.id) {
                Some(slot) => {
                    *slot = resource.clone();
                    Ok(true)
                }
                None => Ok(false),
            }
        }

        fn delete_resource(&self, id: ResourceId) -> RepoResult<bool> {
            let mut resources = Self::lock(&self.resources);
            let before = resources.len();
            resources.retain(|r| r.id != id);
            Ok(resources.len() != before)
        }

        fn list_resources(&self, task_id: TaskId) -> RepoResult<Vec<TaskResource>> {
            let mut resources: Vec<TaskResource> = Self::lock(&self.resources)
                .iter()
                .filter(|r| r.task_id == task_id)
                .cloned()
                .collect();
            resources.sort_by(|a, b| a.sort_order.cmp(&b.sort_order));
            Ok(resources)
        }

        fn list_all_resources(&self) -> RepoResult<Vec<TaskResource>> {
            let mut resources = Self::lock(&self.resources).clone();
            resources.sort_by(|a, b| (a.task_id, &a.sort_order).cmp(&(b.task_id, &b.sort_order)));
            Ok(resources)
        }

        fn insert_update(&self, entry: &TaskUpdateEntry) -> RepoResult<()> {
            Self::lock(&self.updates).push(entry.clone());
            Ok(())
        }

        fn list_updates(&self, task_id: TaskId) -> RepoResult<Vec<TaskUpdateEntry>> {
            let mut updates: Vec<TaskUpdateEntry> = Self::lock(&self.updates)
                .iter()
                .filter(|u| u.task_id == task_id)
                .cloned()
                .collect();
            updates.sort_by_key(|u| u.created_at);
            Ok(updates)
        }
    }

    fn service() -> TaskService {
        let today = NaiveDate::from_ymd_opt(2026, 9, 2).expect("有効な日付");
        TaskService::new(
            Arc::new(MemoryRepository::default()),
            Arc::new(FixedClock::at(today)),
            CoreSettings::default(),
        )
    }

    fn new_task(title: &str) -> NewTask {
        NewTask {
            title: title.to_owned(),
            ..NewTask::default()
        }
    }

    #[test]
    fn create_task_defaults_to_new_status() {
        let service = service();
        let task = service.create_task(new_task("買い物")).unwrap();
        assert_eq!(task.status, TaskStatus::New);
        assert_eq!(task.scheduled, Scheduled::None);
        assert_eq!(task.origin, Origin::User);
        assert!(!task.is_instant);
        assert_eq!(task.sort_order, sort_order::FIRST_KEY);
    }

    #[test]
    fn create_task_rejects_blank_title() {
        let service = service();
        assert!(matches!(
            service.create_task(new_task("   ")),
            Err(CoreError::EmptyTitle)
        ));
    }

    #[test]
    fn create_task_appends_to_the_end_of_the_column() {
        let service = service();
        let first = service.create_task(new_task("1")).unwrap();
        let second = service.create_task(new_task("2")).unwrap();
        assert!(first.sort_order < second.sort_order);
    }

    #[test]
    fn create_task_with_resources_marks_the_first_as_primary() {
        let service = service();
        let task = service
            .create_task(NewTask {
                title: "PR を見る".to_owned(),
                resources: vec![
                    NewResource {
                        kind: ResourceKind::Url,
                        value: "https://example.com/pr/1".to_owned(),
                        label: String::new(),
                        is_primary: false,
                    },
                    NewResource {
                        kind: ResourceKind::File,
                        value: "C:/tmp/memo.txt".to_owned(),
                        label: "メモ".to_owned(),
                        is_primary: false,
                    },
                ],
                ..NewTask::default()
            })
            .unwrap();
        let detail = service.task_detail(task.id).unwrap();
        assert_eq!(detail.resources.len(), 2);
        assert!(detail.resources[0].is_primary);
        assert!(!detail.resources[1].is_primary);
        assert_eq!(detail.card.resource_count, 2);
        assert!(detail.card.primary_resource.is_some());
    }

    #[test]
    fn move_task_maps_columns_to_schedules() {
        let service = service();
        let task = service.create_task(new_task("会議")).unwrap();
        let today = service.today();

        let moved = service
            .move_task(task.id, MoveRequest::to_column(BoardColumn::Today))
            .unwrap();
        assert_eq!(moved.status, TaskStatus::Todo);
        assert_eq!(moved.scheduled, Scheduled::Date(today));

        let moved = service
            .move_task(task.id, MoveRequest::to_column(BoardColumn::NextWeek))
            .unwrap();
        assert_eq!(
            moved.scheduled,
            crate::bucket::scheduled_for_bucket(Bucket::NextWeek, today, WeekStart::Monday)
        );

        let moved = service
            .move_task(task.id, MoveRequest::to_column(BoardColumn::Future))
            .unwrap();
        assert_eq!(moved.scheduled, Scheduled::None);
    }

    #[test]
    fn move_task_between_neighbours_produces_an_in_between_key() {
        let service = service();
        let a = service.create_task(new_task("a")).unwrap();
        let b = service.create_task(new_task("b")).unwrap();
        let c = service.create_task(new_task("c")).unwrap();

        let moved = service
            .move_task(
                c.id,
                MoveRequest {
                    column: BoardColumn::New,
                    prev_id: Some(a.id),
                    next_id: Some(b.id),
                },
            )
            .unwrap();
        assert!(a.sort_order < moved.sort_order);
        assert!(moved.sort_order < b.sort_order);
    }

    #[test]
    fn move_to_done_and_back_manages_done_at() {
        let service = service();
        let task = service.create_task(new_task("片付け")).unwrap();
        let done = service
            .move_task(task.id, MoveRequest::to_column(BoardColumn::Done))
            .unwrap();
        assert_eq!(done.status, TaskStatus::Done);
        assert!(done.done_at.is_some());

        let back = service
            .move_task(task.id, MoveRequest::to_column(BoardColumn::Doing))
            .unwrap();
        assert_eq!(back.status, TaskStatus::Doing);
        assert!(back.done_at.is_none());
    }

    /// フロントエンドが受け取る JSON の形を固定する(引き継ぎ用の契約テスト)。
    #[test]
    fn board_and_detail_json_shape() {
        let service = service();
        let task = service
            .create_task(NewTask {
                title: "契約".to_owned(),
                resources: vec![NewResource {
                    kind: ResourceKind::Url,
                    value: "https://example.com".to_owned(),
                    label: String::new(),
                    is_primary: true,
                }],
                ..NewTask::default()
            })
            .unwrap();
        service
            .move_task(task.id, MoveRequest::to_column(BoardColumn::ThisWeek))
            .unwrap();

        let board = serde_json::to_value(service.board().unwrap()).unwrap();
        assert_eq!(board["today"], "2026-09-02");
        assert_eq!(board["weekStart"], "monday");
        let card = &board["columns"]["thisWeek"][0];
        // TaskCard は Task を平坦化して持つ。
        assert_eq!(card["id"], task.id.to_string());
        assert_eq!(card["title"], "契約");
        assert_eq!(card["status"], "todo");
        assert_eq!(card["bucket"], "thisWeek");
        assert_eq!(card["scheduled"]["kind"], "week");
        assert_eq!(card["isInstant"], false);
        assert_eq!(card["origin"], "user");
        assert_eq!(card["childCount"], 0);
        assert_eq!(card["resourceCount"], 1);
        assert_eq!(card["primaryResource"]["kind"], "url");
        assert!(card["parentId"].is_null());

        let detail = serde_json::to_value(service.task_detail(task.id).unwrap()).unwrap();
        assert_eq!(detail["id"], task.id.to_string());
        assert_eq!(detail["resources"][0]["isPrimary"], true);
        assert!(detail["updates"].is_array());
        assert!(detail["children"].is_array());
        assert!(detail["parent"].is_null());
    }

    #[test]
    fn complete_task_is_idempotent() {
        let service = service();
        let task = service.create_task(new_task("提出")).unwrap();
        let first = service.complete_task(task.id).unwrap();
        let second = service.complete_task(task.id).unwrap();
        assert_eq!(first.done_at, second.done_at);
        assert_eq!(second.status, TaskStatus::Done);
    }

    #[test]
    fn promote_task_clears_the_instant_flag() {
        let service = service();
        let task = service
            .create_task(NewTask {
                title: "PR 確認".to_owned(),
                is_instant: true,
                ..NewTask::default()
            })
            .unwrap();
        let promoted = service.promote_task(task.id, None).unwrap();
        assert!(!promoted.is_instant);
        assert_eq!(promoted.status, TaskStatus::Todo);
        assert_eq!(promoted.scheduled, Scheduled::Date(service.today()));

        assert!(matches!(
            service.promote_task(task.id, None),
            Err(CoreError::NotInstant(_))
        ));
    }

    #[test]
    fn add_and_remove_resources_maintain_a_single_primary() {
        let service = service();
        let task = service.create_task(new_task("資料")).unwrap();
        let first = service
            .add_resource(
                task.id,
                NewResource {
                    kind: ResourceKind::Url,
                    value: "https://example.com/a".to_owned(),
                    label: String::new(),
                    is_primary: false,
                },
            )
            .unwrap();
        assert!(first.is_primary, "最初のリソースは主リソースになる");

        let second = service
            .add_resource(
                task.id,
                NewResource {
                    kind: ResourceKind::Url,
                    value: "https://example.com/b".to_owned(),
                    label: String::new(),
                    is_primary: true,
                },
            )
            .unwrap();
        let detail = service.task_detail(task.id).unwrap();
        let primaries: Vec<_> = detail.resources.iter().filter(|r| r.is_primary).collect();
        assert_eq!(primaries.len(), 1);
        assert_eq!(primaries[0].id, second.id);

        service.remove_resource(task.id, first.id).unwrap();
        assert_eq!(service.task_detail(task.id).unwrap().resources.len(), 1);
        assert!(matches!(
            service.remove_resource(task.id, first.id),
            Err(CoreError::ResourceNotFound(_))
        ));
    }

    #[test]
    fn add_task_update_appends_history() {
        let service = service();
        let task = service.create_task(new_task("調査")).unwrap();
        service
            .add_task_update(task.id, "調べ始めた", Origin::User)
            .unwrap();
        service
            .add_task_update(task.id, "半分終わった", Origin::Ai)
            .unwrap();
        let detail = service.task_detail(task.id).unwrap();
        assert_eq!(detail.updates.len(), 2);
        assert_eq!(detail.updates[0].body, "調べ始めた");
        assert_eq!(detail.updates[1].origin, Origin::Ai);
    }

    #[test]
    fn set_parent_rejects_self_and_cycles() {
        let service = service();
        let root = service.create_task(new_task("親")).unwrap();
        let child = service.create_task(new_task("子")).unwrap();
        let grandchild = service.create_task(new_task("孫")).unwrap();

        service.set_parent(child.id, Some(root.id)).unwrap();
        service.set_parent(grandchild.id, Some(child.id)).unwrap();

        assert!(matches!(
            service.set_parent(root.id, Some(root.id)),
            Err(CoreError::SelfParent(_))
        ));
        assert!(matches!(
            service.set_parent(root.id, Some(grandchild.id)),
            Err(CoreError::ParentCycle { .. })
        ));

        // 解除できる。
        let cleared = service.set_parent(grandchild.id, None).unwrap();
        assert_eq!(cleared.parent_id, None);
    }

    #[test]
    fn set_parent_reports_missing_tasks() {
        let service = service();
        let task = service.create_task(new_task("親なし")).unwrap();
        let missing = TaskId::new();
        assert!(matches!(
            service.set_parent(task.id, Some(missing)),
            Err(CoreError::TaskNotFound(_))
        ));
        assert!(matches!(
            service.task_detail(missing),
            Err(CoreError::TaskNotFound(_))
        ));
    }

    #[test]
    fn board_groups_tasks_into_derived_buckets() {
        let service = service();
        let today = service.today();

        let inbox = service.create_task(new_task("受信箱")).unwrap();
        let a = service.create_task(new_task("今日")).unwrap();
        let b = service.create_task(new_task("明日")).unwrap();
        let c = service.create_task(new_task("今週")).unwrap();
        let d = service.create_task(new_task("来週")).unwrap();
        let e = service.create_task(new_task("いつか")).unwrap();
        let f = service.create_task(new_task("着手中")).unwrap();
        let g = service.create_task(new_task("完了")).unwrap();

        service
            .move_task(a.id, MoveRequest::to_column(BoardColumn::Today))
            .unwrap();
        service
            .move_task(b.id, MoveRequest::to_column(BoardColumn::Tomorrow))
            .unwrap();
        service
            .move_task(c.id, MoveRequest::to_column(BoardColumn::ThisWeek))
            .unwrap();
        service
            .move_task(d.id, MoveRequest::to_column(BoardColumn::NextWeek))
            .unwrap();
        service
            .move_task(e.id, MoveRequest::to_column(BoardColumn::Future))
            .unwrap();
        service
            .move_task(f.id, MoveRequest::to_column(BoardColumn::Doing))
            .unwrap();
        service.complete_task(g.id).unwrap();

        let board = service.board().unwrap();
        assert_eq!(board.today, today);
        assert_eq!(board.week_start, WeekStart::Monday);
        assert_eq!(board.columns.new.len(), 1);
        assert_eq!(board.columns.new[0].task.id, inbox.id);
        assert_eq!(board.columns.today.len(), 1);
        assert_eq!(board.columns.tomorrow.len(), 1);
        assert_eq!(board.columns.this_week.len(), 1);
        assert_eq!(board.columns.next_week.len(), 1);
        assert_eq!(board.columns.future.len(), 1);
        assert_eq!(board.columns.doing.len(), 1);
        assert_eq!(board.columns.done.len(), 1);
        assert_eq!(board.columns.today[0].bucket, Some(Bucket::Today));
        assert_eq!(board.columns.new[0].bucket, None);
    }

    #[test]
    fn board_counts_children_and_resources() {
        let service = service();
        let parent = service.create_task(new_task("親")).unwrap();
        let child = service.create_task(new_task("子")).unwrap();
        service.set_parent(child.id, Some(parent.id)).unwrap();
        service
            .add_resource(
                parent.id,
                NewResource {
                    kind: ResourceKind::Url,
                    value: "https://example.com".to_owned(),
                    label: String::new(),
                    is_primary: true,
                },
            )
            .unwrap();

        let board = service.board().unwrap();
        let card = board
            .columns
            .new
            .iter()
            .find(|c| c.task.id == parent.id)
            .expect("親タスクがボードにある");
        assert_eq!(card.child_count, 1);
        assert_eq!(card.resource_count, 1);
        assert!(card.primary_resource.is_some());
    }

    #[test]
    fn board_columns_are_sorted_by_sort_order() {
        let service = service();
        for i in 0..5 {
            service.create_task(new_task(&format!("t{i}"))).unwrap();
        }
        let board = service.board().unwrap();
        let keys: Vec<&str> = board
            .columns
            .new
            .iter()
            .map(|c| c.task.sort_order.as_str())
            .collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted);
    }

    #[test]
    fn overdue_dates_show_up_in_the_today_column() {
        let service = service();
        let task = service.create_task(new_task("遅延")).unwrap();
        // Todo にしてから、過ぎた日付へ予定を書き換える。
        service
            .move_task(task.id, MoveRequest::to_column(BoardColumn::NextWeek))
            .unwrap();
        service
            .update_task(
                task.id,
                TaskPatch {
                    scheduled: Some(Scheduled::Date(
                        NaiveDate::from_ymd_opt(2026, 1, 1).expect("有効な日付"),
                    )),
                    ..TaskPatch::default()
                },
            )
            .unwrap();

        let board = service.board().unwrap();
        assert!(board.columns.next_week.is_empty());
        assert_eq!(board.columns.today.len(), 1);
        assert_eq!(board.columns.today[0].task.id, task.id);
        assert_eq!(board.columns.today[0].bucket, Some(Bucket::Today));
    }

    #[test]
    fn non_todo_tasks_have_no_bucket() {
        let service = service();
        let task = service.create_task(new_task("着手中")).unwrap();
        service
            .move_task(task.id, MoveRequest::to_column(BoardColumn::Doing))
            .unwrap();
        let board = service.board().unwrap();
        assert_eq!(board.columns.doing[0].bucket, None);
    }

    #[test]
    fn update_task_patches_only_given_fields() {
        let service = service();
        let task = service
            .create_task(NewTask {
                title: "元タイトル".to_owned(),
                description: "元の詳細".to_owned(),
                deadline: Some(Utc::now()),
                ..NewTask::default()
            })
            .unwrap();

        let updated = service
            .update_task(
                task.id,
                TaskPatch {
                    title: Some("新タイトル".to_owned()),
                    ..TaskPatch::default()
                },
            )
            .unwrap();
        assert_eq!(updated.title, "新タイトル");
        assert_eq!(updated.description, "元の詳細");
        assert!(updated.deadline.is_some());

        let cleared = service
            .update_task(
                task.id,
                TaskPatch {
                    clear_deadline: true,
                    ..TaskPatch::default()
                },
            )
            .unwrap();
        assert!(cleared.deadline.is_none());
    }

    #[tokio::test]
    async fn domain_events_are_broadcast() {
        let service = service();
        let mut rx = service.subscribe();
        let task = service.create_task(new_task("イベント")).unwrap();
        assert_eq!(
            rx.recv().await.unwrap(),
            DomainEvent::TaskCreated { task_id: task.id }
        );

        service
            .move_task(task.id, MoveRequest::to_column(BoardColumn::Today))
            .unwrap();
        assert_eq!(
            rx.recv().await.unwrap(),
            DomainEvent::TaskMoved {
                task_id: task.id,
                status: TaskStatus::Todo,
                bucket: Some(Bucket::Today),
            }
        );

        service.complete_task(task.id).unwrap();
        assert!(matches!(
            rx.recv().await.unwrap(),
            DomainEvent::TaskMoved { .. }
        ));
        assert_eq!(
            rx.recv().await.unwrap(),
            DomainEvent::TaskCompleted { task_id: task.id }
        );

        let date = NaiveDate::from_ymd_opt(2026, 9, 3).unwrap();
        service.notify_day_changed(date);
        assert_eq!(rx.recv().await.unwrap(), DomainEvent::DayChanged { date });
    }

    #[test]
    fn settings_can_be_replaced_and_affect_bucket_derivation() {
        let service = service();
        assert_eq!(service.settings().week_start, WeekStart::Monday);
        service.set_settings(CoreSettings {
            week_start: WeekStart::Sunday,
            ..CoreSettings::default()
        });
        assert_eq!(service.settings().week_start, WeekStart::Sunday);

        let task = service.create_task(new_task("週")).unwrap();
        service
            .move_task(task.id, MoveRequest::to_column(BoardColumn::ThisWeek))
            .unwrap();
        let board = service.board().unwrap();
        assert_eq!(board.week_start, WeekStart::Sunday);
        assert_eq!(board.columns.this_week.len(), 1);
    }
}
