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

use crate::bucket::{bucket_for, BoardColumn, Bucket};
use crate::clock::Clock;
use crate::error::{CoreError, CoreResult};
use crate::events::DomainEvent;
use crate::model::{
    Origin, ResourceId, ResourceKind, Scheduled, Task, TaskId, TaskResource, TaskStatus,
    TaskUpdateEntry, UpdateId,
};
use crate::repository::TaskRepository;
use crate::settings::{BoardSettings, WeekStart};
use crate::sort_order;

/// ドメインイベントのブロードキャスト容量。
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// [`TaskService::list_archived_done`] が 1 回で返す最大件数。
///
/// 「過去の完了」は掘り返す一覧であって作業対象ではないので、ページングは持たず
/// 直近だけを返す。総件数は [`ArchivedDone::total`] で分かる。
pub const ARCHIVED_DONE_LIMIT: usize = 200;

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
    /// Watching 列(外部の変化待ち)。
    pub watching: Vec<TaskCard>,
    /// Doing 列。
    pub doing: Vec<TaskCard>,
    /// Done 列。
    pub done: Vec<TaskCard>,
}

impl BoardColumns {
    /// 列と、その列のタスクを左から順に列挙する。
    pub fn iter(&self) -> impl Iterator<Item = (BoardColumn, &[TaskCard])> {
        [
            (BoardColumn::New, self.new.as_slice()),
            (BoardColumn::Today, self.today.as_slice()),
            (BoardColumn::Tomorrow, self.tomorrow.as_slice()),
            (BoardColumn::ThisWeek, self.this_week.as_slice()),
            (BoardColumn::NextWeek, self.next_week.as_slice()),
            (BoardColumn::Future, self.future.as_slice()),
            (BoardColumn::Watching, self.watching.as_slice()),
            (BoardColumn::Doing, self.doing.as_slice()),
            (BoardColumn::Done, self.done.as_slice()),
        ]
        .into_iter()
    }

    /// 指定した列のリストを借りる。
    fn column_mut(&mut self, column: BoardColumn) -> &mut Vec<TaskCard> {
        match column {
            BoardColumn::New => &mut self.new,
            BoardColumn::Today => &mut self.today,
            BoardColumn::Tomorrow => &mut self.tomorrow,
            BoardColumn::ThisWeek => &mut self.this_week,
            BoardColumn::NextWeek => &mut self.next_week,
            BoardColumn::Future => &mut self.future,
            BoardColumn::Watching => &mut self.watching,
            BoardColumn::Doing => &mut self.doing,
            BoardColumn::Done => &mut self.done,
        }
    }
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
    /// 前日以前に完了したタスクの件数。
    ///
    /// [`board`](TaskService::board) の Done 列は**今日完了した分だけ**なので、
    /// それ以前の完了はここに件数としてだけ現れる。中身は
    /// [`list_archived_done`](TaskService::list_archived_done) で取る。
    pub archived_done_count: usize,
}

/// 「過去の完了」一覧([`list_archived_done`](TaskService::list_archived_done))。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedDone {
    /// 完了が新しい順。最大 [`limit`](Self::limit) 件。
    pub tasks: Vec<TaskCard>,
    /// 条件に合う総件数(`tasks` が切り詰められていても実数)。
    pub total: usize,
    /// 適用した上限。`total > limit` なら古い分は返っていない。
    pub limit: usize,
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
///
/// 設定はボード表示に必要な [`BoardSettings`] だけを持つ。設定全体
/// ([`CoreSettings`](crate::settings::CoreSettings))の保持・永続化・配布は
/// アプリ(シェル)側の責務で、サービスはその変更を
/// [`notify_settings_changed`](Self::notify_settings_changed) で中継するだけ。
pub struct TaskService {
    repo: Arc<dyn TaskRepository>,
    clock: Arc<dyn Clock>,
    board: RwLock<BoardSettings>,
    /// 「読んで書く」操作を直列化するためのロック。
    write_lock: Mutex<()>,
    events: broadcast::Sender<DomainEvent>,
}

impl std::fmt::Debug for TaskService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskService")
            .field("board", &self.board_settings())
            .finish_non_exhaustive()
    }
}

impl TaskService {
    /// サービスを構築する。
    #[must_use]
    pub fn new(repo: Arc<dyn TaskRepository>, clock: Arc<dyn Clock>, board: BoardSettings) -> Self {
        let (events, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            repo,
            clock,
            board: RwLock::new(board),
            write_lock: Mutex::new(()),
            events,
        }
    }

    /// ドメインイベントを購読する。
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<DomainEvent> {
        self.events.subscribe()
    }

    /// 現在のボード設定を返す。
    #[must_use]
    pub fn board_settings(&self) -> BoardSettings {
        *self
            .board
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// バケット導出に使う週開始曜日。
    #[must_use]
    pub fn week_start(&self) -> WeekStart {
        self.board_settings().week_start
    }

    /// ボード設定を差し替える。イベントは発行しない。
    ///
    /// 設定全体の保存が終わったら [`notify_settings_changed`](Self::notify_settings_changed)
    /// を呼んで購読者へ知らせること。
    pub fn set_board_settings(&self, board: BoardSettings) {
        // 毒された場合も更新を続行する(board_settings / lock_writes と同じ方針)。
        *self
            .board
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = board;
    }

    /// 設定が変わったことを購読者へ知らせる([`DomainEvent::SettingsChanged`])。
    pub fn notify_settings_changed(&self) {
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
    /// **Done 列には「今日完了した分」しか入らない。** 前日以前の完了は
    /// [`Board::archived_done_count`] に件数としてだけ現れ、中身は
    /// [`list_archived_done`](Self::list_archived_done) で取る。
    /// バケット導出と同じで、これは**保存された状態ではなく表示時の導出**なので、
    /// 日付が変われば([`DomainEvent::DayChanged`] を受けた再取得で)自動的に消える。
    ///
    /// # Errors
    /// 永続化層のエラー。
    pub fn board(&self) -> CoreResult<Board> {
        self.build_board(false)
    }

    /// Done を絞り込まないボード。
    ///
    /// [`board`](Self::board) が今日の完了だけを見せるのに対し、こちらは
    /// 完了したタスクを**すべて** Done 列に入れる。MCP のように「ボードの見た目」ではなく
    /// 「いま何があるか」を知りたい呼び出し側のためのもの。
    /// [`Board::archived_done_count`] の意味は [`board`](Self::board) と同じ
    /// (= そのうち前日以前に完了した件数)。
    ///
    /// # Errors
    /// 永続化層のエラー。
    pub fn full_board(&self) -> CoreResult<Board> {
        self.build_board(true)
    }

    fn build_board(&self, include_archived_done: bool) -> CoreResult<Board> {
        let today = self.clock.today();
        let week_start = self.week_start();

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
            columns
                .column_mut(BoardColumn::of(card.task.status, card.bucket))
                .push(card);
        }

        // Done 列は「今日完了した分」だけ。件数は絞り込みの前に数える。
        let archived_done_count = columns
            .done
            .iter()
            .filter(|card| self.is_archived_done(&card.task, today))
            .count();
        if !include_archived_done {
            columns
                .done
                .retain(|card| !self.is_archived_done(&card.task, today));
        }

        Ok(Board {
            today,
            week_start,
            columns,
            archived_done_count,
        })
    }

    /// 前日以前に完了したか(= ボードの Done 列から外れるか)。
    ///
    /// `done_at` を持たない完了タスク(サービス経由では作れないが、古いデータには
    /// ありうる)は隠さない。いつ終わったか分からないものを黙って消さないため。
    fn is_archived_done(&self, task: &Task, today: NaiveDate) -> bool {
        task.done_at
            .is_some_and(|at| self.clock.local_date(at) < today)
    }

    /// 前日以前に完了したタスクを、完了が新しい順に返す。
    ///
    /// ボードの Done 列から外れた分の一覧。1 回で返すのは
    /// [`ARCHIVED_DONE_LIMIT`] 件までで、それを超える分は返さない
    /// (`total` に総件数が入る。ページングは提供しない)。
    /// 削除済みは含まない。復元 UI と同じく、子タスク数・リソース数は集計しない。
    ///
    /// # Errors
    /// 永続化層のエラー。
    pub fn list_archived_done(&self) -> CoreResult<ArchivedDone> {
        let today = self.clock.today();
        let week_start = self.week_start();
        // 「ローカル日付が今日より前」= 「今日の 0:00 (UTC 換算) より前」。
        let before = self.clock.local_day_start(today);

        let total = self.repo.count_done_before(before)?;
        let empty_counts = HashMap::new();
        let empty_resources = HashMap::new();
        let tasks = self
            .repo
            .list_done_before(before, ARCHIVED_DONE_LIMIT)?
            .into_iter()
            .map(|task| build_card(task, today, week_start, &empty_counts, &empty_resources))
            .collect();

        Ok(ArchivedDone {
            tasks,
            total,
            limit: ARCHIVED_DONE_LIMIT,
        })
    }

    /// タスク詳細(リソース・履歴・親子込み)を返す。
    ///
    /// # Errors
    /// タスクが存在しない場合、または永続化層のエラー。
    pub fn task_detail(&self, id: TaskId) -> CoreResult<TaskDetail> {
        let today = self.clock.today();
        let week_start = self.week_start();
        let task = self.require_task(id)?;

        let resources = self.repo.list_resources(id)?;
        let updates = self.repo.list_updates(id)?;
        let children = self.repo.list_children(id)?;
        // 親子リンクは削除しても保持するので、削除済みの親はここで落とすだけにする
        // (復元すればリンクが自然に戻る)。子は list_children が既に除外している。
        let parent = match task.parent_id {
            Some(parent_id) => self
                .repo
                .find_task(parent_id)?
                .filter(|parent| !parent.is_deleted()),
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

    /// タスクを 1 件取得する。削除済みのタスクは `None` として扱う。
    ///
    /// 削除済みも含めて見たい場合は [`list_deleted`](Self::list_deleted) を使う。
    ///
    /// # Errors
    /// 永続化層のエラー。
    pub fn find_task(&self, id: TaskId) -> CoreResult<Option<Task>> {
        Ok(self.repo.find_task(id)?.filter(|task| !task.is_deleted()))
    }

    /// 指定ステータスの(削除済みでない)タスクを返す。
    ///
    /// # Errors
    /// 永続化層のエラー。
    pub fn list_by_status(&self, status: TaskStatus) -> CoreResult<Vec<Task>> {
        Ok(self.repo.list_tasks_by_status(status)?)
    }

    /// 削除済みタスクを、新しく消したものから順に返す。
    ///
    /// 復元 UI 用の一覧なので、子タスク数・リソース数は集計しない(いずれも 0)。
    /// 削除時のステータスと予定はそのまま残っているため、`status` と `bucket` から
    /// 「元どこにいたか」を示せる。
    ///
    /// # Errors
    /// 永続化層のエラー。
    pub fn list_deleted(&self) -> CoreResult<Vec<TaskCard>> {
        let today = self.clock.today();
        let week_start = self.week_start();
        let empty_counts = HashMap::new();
        let empty_resources = HashMap::new();
        Ok(self
            .repo
            .list_deleted_tasks()?
            .into_iter()
            .map(|task| build_card(task, today, week_start, &empty_counts, &empty_resources))
            .collect())
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
            deleted_at: None,
        };
        // 主リソースはちょうど 1 つ。明示指定が無ければ先頭を主リソースにする。
        let primary_index = input
            .resources
            .iter()
            .position(|resource| resource.is_primary)
            .unwrap_or(0);
        let mut resources = Vec::with_capacity(input.resources.len());
        let mut previous_key: Option<String> = None;
        for (index, resource) in input.resources.into_iter().enumerate() {
            let key = sort_order::generate_key_between(previous_key.as_deref(), None)?;
            resources.push(TaskResource {
                id: ResourceId::new(),
                task_id: task.id,
                kind: resource.kind,
                value: resource.value,
                label: resource.label,
                is_primary: index == primary_index,
                sort_order: key.clone(),
                created_at: now,
            });
            previous_key = Some(key);
        }

        // タスクとリソースは 1 つのトランザクションで入れる(中間状態を残さない)。
        self.repo.insert_task_with_resources(&task, &resources)?;

        self.emit(DomainEvent::TaskCreated { task_id: task.id });
        // 外部 origin が子タスクを足したら、監視中の親を起こす。
        if let Some(parent_id) = task.parent_id {
            self.wake_task(parent_id, &task.origin)?;
        }
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
        let week_start = self.week_start();

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
        let week_start = self.week_start();
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
    /// **監視中 ([`TaskStatus::Watching`]) のタスクに、ユーザー以外の origin
    /// (`mcp` / `ai` / `plugin:*` / `system`)で追記すると起床する**
    /// ([`wake`](Self::wake) 参照)。「外の変化を待っていたタスクが、変化を知らされて
    /// New に戻る」という Watching の主経路。
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
        // 起床も同じ 1 回の書き込みに載せる(履歴だけ入って移動が漏れる隙を作らない)。
        let woken = self.wake(&mut task, &entry.origin)?;
        self.persist(&task)?;
        self.emit(DomainEvent::TaskUpdateAdded { task_id: id });
        if woken {
            self.emit_woken(id);
        }
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
        // 既存の主リソースの解除と挿入は原子的に行う(主リソースが 0 件の瞬間を作らない)。
        self.repo.replace_primary_and_insert(&resource)?;
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

    /// タスクを削除する(ソフトデリート)。
    ///
    /// `deleted_at` を立てるだけで行は消さない。ボード・一覧・子タスクからは
    /// 消えるが、親子リンクとリソース・履歴はそのまま残る。**子タスクへは
    /// カスケードしない**(子は親を失って見えるだけで、親を復元すれば戻る)。
    /// すでに削除済みなら何もせず成功する(冪等)。
    ///
    /// # Errors
    /// タスクが存在しない、または永続化層のエラー。
    pub fn delete_task(&self, id: TaskId) -> CoreResult<Task> {
        let _guard = self.lock_writes();
        let mut task = self.find_any(id)?;
        if task.is_deleted() {
            return Ok(task);
        }
        task.updated_at = self.clock.now();
        task.deleted_at = Some(task.updated_at);

        self.persist(&task)?;
        self.emit(DomainEvent::TaskDeleted { task_id: id });
        Ok(task)
    }

    /// 削除済みタスクを復元する。
    ///
    /// `deleted_at` を `None` に戻し、**現在のステータス列の末尾へ並び直す**。
    /// 削除中に同じ `sort_order` を持つタスクが増えている可能性があるため、
    /// 古いキーをそのまま戻すことはしない。
    /// 生存中のタスクに対しては何もせず成功する(冪等)。
    ///
    /// # Errors
    /// タスクが存在しない、並び順キーの生成に失敗、または永続化層のエラー。
    pub fn restore_task(&self, id: TaskId) -> CoreResult<Task> {
        let _guard = self.lock_writes();
        let mut task = self.find_any(id)?;
        if !task.is_deleted() {
            return Ok(task);
        }
        // end_key は削除済みを数えないので、deleted_at を落とす前に計算してよい。
        task.sort_order = self.end_key(task.status)?;
        task.updated_at = self.clock.now();
        task.deleted_at = None;

        self.persist(&task)?;
        self.emit(DomainEvent::TaskRestored { task_id: id });
        Ok(task)
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

    // ---- 起床 (Watching) ----

    /// 監視中のタスクを New へ起こす。起こしたら `true`。
    ///
    /// docs/data-model.md の規則そのもの。
    ///
    /// - 起こすのは **`status == Watching`** かつ **origin がユーザー以外**のときだけ。
    ///   ユーザー自身の編集で勝手に起きてはいけない。
    /// - **`scheduled` は保持する**(Watching へ入れる前の予定を失わない)。
    /// - 並び順は New 列の末尾。
    ///
    /// 永続化とイベント発行はしない。呼び出し側が同じ 1 回の [`persist`](Self::persist) に
    /// 載せ、成功したら [`emit_woken`](Self::emit_woken) を呼ぶ。
    /// 書き込みロックを取った状態で呼ぶこと。
    fn wake(&self, task: &mut Task, origin: &Origin) -> CoreResult<bool> {
        if task.status != TaskStatus::Watching || origin.is_user() {
            return Ok(false);
        }
        task.status = TaskStatus::New;
        task.sort_order = self.end_key(TaskStatus::New)?;
        task.done_at = None;
        Ok(true)
    }

    /// 別のタスク(典型的には親)を起床させ、必要なら永続化とイベント発行まで行う。
    ///
    /// 監視中でない・origin がユーザー・そもそも起きる必要がない場合は何もしない(冪等)。
    /// 書き込みロックを取った状態で呼ぶこと。
    fn wake_task(&self, id: TaskId, origin: &Origin) -> CoreResult<()> {
        if origin.is_user() {
            return Ok(());
        }
        let mut task = self.require_task(id)?;
        if !self.wake(&mut task, origin)? {
            return Ok(());
        }
        task.updated_at = self.clock.now();
        self.persist(&task)?;
        self.emit_woken(id);
        Ok(())
    }

    /// 起床を購読者へ知らせる。`TaskWoken` の直後に New 列への `TaskMoved` を続ける
    /// (UI・オーバーレイは移動イベントで再取得する)。
    fn emit_woken(&self, id: TaskId) {
        self.emit(DomainEvent::TaskWoken { task_id: id });
        self.emit(DomainEvent::TaskMoved {
            task_id: id,
            status: TaskStatus::New,
            bucket: None,
        });
    }

    // ---- 内部ヘルパ ----

    fn lock_writes(&self) -> std::sync::MutexGuard<'_, ()> {
        // 毒された場合も処理を継続する(この Mutex は `()` を守るだけで状態を持たない)。
        self.write_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn emit(&self, event: DomainEvent) {
        // 購読者がいない場合のエラーは無視してよい。
        let _ = self.events.send(event);
    }

    /// 通常操作の対象となるタスクを取り出す。
    ///
    /// **削除済みのタスクはここで [`CoreError::TaskDeleted`] として弾く。**
    /// 削除済みへの更新・移動・完了などを取りこぼしなく拒否するため、
    /// 通常のユースケースは必ずこれを通す(復元だけが [`find_any`](Self::find_any) を使う)。
    fn require_task(&self, id: TaskId) -> CoreResult<Task> {
        let task = self.find_any(id)?;
        if task.is_deleted() {
            return Err(CoreError::TaskDeleted(id));
        }
        Ok(task)
    }

    /// 削除済みも含めてタスクを取り出す。削除・復元だけが使う。
    fn find_any(&self, id: TaskId) -> CoreResult<Task> {
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

        /// `before` より前に完了した(削除済みでない)タスク。SQL 側の条件と同じ。
        fn done_before(&self, before: DateTime<Utc>) -> Vec<Task> {
            Self::lock(&self.tasks)
                .iter()
                .filter(|t| {
                    t.status == TaskStatus::Done
                        && !t.is_deleted()
                        && t.done_at.is_some_and(|at| at < before)
                })
                .cloned()
                .collect()
        }
    }

    impl TaskRepository for MemoryRepository {
        fn insert_task_with_resources(
            &self,
            task: &Task,
            resources: &[TaskResource],
        ) -> RepoResult<()> {
            let mut tasks = Self::lock(&self.tasks);
            if tasks.iter().any(|t| t.id == task.id) {
                return Err(RepositoryError::message("重複した ID"));
            }
            tasks.push(task.clone());
            Self::lock(&self.resources).extend(resources.iter().cloned());
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

        // 通常クエリからの削除済み除外はリポジトリ実装の責務
        // (SQLite 実装の `WHERE deleted_at IS NULL` に対応する)。
        fn list_tasks(&self) -> RepoResult<Vec<Task>> {
            let mut tasks: Vec<Task> = Self::lock(&self.tasks)
                .iter()
                .filter(|t| !t.is_deleted())
                .cloned()
                .collect();
            tasks.sort_by(|a, b| a.sort_order.cmp(&b.sort_order));
            Ok(tasks)
        }

        fn list_tasks_by_status(&self, status: TaskStatus) -> RepoResult<Vec<Task>> {
            let mut tasks: Vec<Task> = Self::lock(&self.tasks)
                .iter()
                .filter(|t| t.status == status && !t.is_deleted())
                .cloned()
                .collect();
            tasks.sort_by(|a, b| a.sort_order.cmp(&b.sort_order));
            Ok(tasks)
        }

        fn list_children(&self, parent_id: TaskId) -> RepoResult<Vec<Task>> {
            let mut tasks: Vec<Task> = Self::lock(&self.tasks)
                .iter()
                .filter(|t| t.parent_id == Some(parent_id) && !t.is_deleted())
                .cloned()
                .collect();
            tasks.sort_by(|a, b| a.sort_order.cmp(&b.sort_order));
            Ok(tasks)
        }

        fn list_deleted_tasks(&self) -> RepoResult<Vec<Task>> {
            let mut tasks: Vec<Task> = Self::lock(&self.tasks)
                .iter()
                .filter(|t| t.is_deleted())
                .cloned()
                .collect();
            // 新しく消したものが先。同時刻なら id で安定させる。
            tasks.sort_by(|a, b| {
                b.deleted_at
                    .cmp(&a.deleted_at)
                    .then_with(|| a.id.cmp(&b.id))
            });
            Ok(tasks)
        }

        fn list_done_before(&self, before: DateTime<Utc>, limit: usize) -> RepoResult<Vec<Task>> {
            let mut tasks = self.done_before(before);
            // 完了が新しいものが先。同時刻なら id で安定させる。
            tasks.sort_by(|a, b| b.done_at.cmp(&a.done_at).then_with(|| b.id.cmp(&a.id)));
            tasks.truncate(limit);
            Ok(tasks)
        }

        fn count_done_before(&self, before: DateTime<Utc>) -> RepoResult<usize> {
            Ok(self.done_before(before).len())
        }

        fn replace_primary_and_insert(&self, resource: &TaskResource) -> RepoResult<()> {
            let mut resources = Self::lock(&self.resources);
            if resource.is_primary {
                for existing in resources
                    .iter_mut()
                    .filter(|r| r.task_id == resource.task_id)
                {
                    existing.is_primary = false;
                }
            }
            resources.push(resource.clone());
            Ok(())
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
            let alive: HashSet<TaskId> = Self::lock(&self.tasks)
                .iter()
                .filter(|t| !t.is_deleted())
                .map(|t| t.id)
                .collect();
            let mut resources: Vec<TaskResource> = Self::lock(&self.resources)
                .iter()
                .filter(|r| alive.contains(&r.task_id))
                .cloned()
                .collect();
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
            BoardSettings::default(),
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
        // Done 列は今日完了した分だけ。前日以前の分は件数としてだけ載る。
        assert_eq!(board["archivedDoneCount"], 0);
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
        // ソフトデリート (v2)。生存しているタスクは null。
        assert!(card["deletedAt"].is_null());

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
        assert!(board.columns.watching.is_empty());
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
        assert_eq!(service.week_start(), WeekStart::Monday);
        service.set_board_settings(BoardSettings {
            week_start: WeekStart::Sunday,
        });
        assert_eq!(service.board_settings().week_start, WeekStart::Sunday);

        let task = service.create_task(new_task("週")).unwrap();
        service
            .move_task(task.id, MoveRequest::to_column(BoardColumn::ThisWeek))
            .unwrap();
        let board = service.board().unwrap();
        assert_eq!(board.week_start, WeekStart::Sunday);
        assert_eq!(board.columns.this_week.len(), 1);
    }

    #[tokio::test]
    async fn settings_changes_are_broadcast_on_demand() {
        let service = service();
        let mut rx = service.subscribe();
        // 保持している値の差し替えだけではイベントを出さない。
        service.set_board_settings(BoardSettings {
            week_start: WeekStart::Sunday,
        });
        service.notify_settings_changed();
        assert_eq!(rx.recv().await.unwrap(), DomainEvent::SettingsChanged);
    }

    // ---- ソフトデリート ----

    #[test]
    fn delete_task_hides_it_from_the_board_and_lists_it_as_deleted() {
        let service = service();
        let keep = service.create_task(new_task("残す")).unwrap();
        let drop = service.create_task(new_task("消す")).unwrap();

        let deleted = service.delete_task(drop.id).unwrap();
        assert!(deleted.is_deleted());
        assert_eq!(deleted.deleted_at, Some(service.clock.now()));

        let board = service.board().unwrap();
        let ids: Vec<TaskId> = board.columns.new.iter().map(|c| c.task.id).collect();
        assert_eq!(ids, [keep.id]);

        // 通常の参照系からも消える。
        assert!(service.find_task(drop.id).unwrap().is_none());
        assert_eq!(service.list_by_status(TaskStatus::New).unwrap().len(), 1);

        // 削除済み一覧には出る。元のステータス・予定は残っている。
        let deleted_cards = service.list_deleted().unwrap();
        assert_eq!(deleted_cards.len(), 1);
        assert_eq!(deleted_cards[0].task.id, drop.id);
        assert_eq!(deleted_cards[0].task.status, TaskStatus::New);
        assert!(deleted_cards[0].task.deleted_at.is_some());
    }

    #[test]
    fn deleted_tasks_drop_out_of_parent_and_child_views() {
        let service = service();
        let parent = service.create_task(new_task("親")).unwrap();
        let child = service.create_task(new_task("子")).unwrap();
        service.set_parent(child.id, Some(parent.id)).unwrap();

        // 親を消すと、子から親が見えなくなる(リンク自体は残る)。
        service.delete_task(parent.id).unwrap();
        let detail = service.task_detail(child.id).unwrap();
        assert!(detail.parent.is_none());
        assert_eq!(detail.card.task.parent_id, Some(parent.id));
        // ボードの子タスク数にも数えない。
        let board = service.board().unwrap();
        assert_eq!(board.columns.new.len(), 1);

        // 親を戻すとリンクも戻る。
        service.restore_task(parent.id).unwrap();
        assert_eq!(
            service
                .task_detail(child.id)
                .unwrap()
                .parent
                .map(|p| p.task.id),
            Some(parent.id)
        );

        // 子を消すと、親の子タスク一覧から消える(カスケードはしない)。
        service.delete_task(child.id).unwrap();
        let detail = service.task_detail(parent.id).unwrap();
        assert!(detail.children.is_empty());
        assert_eq!(detail.card.child_count, 0);
        assert!(service.task_detail(child.id).is_err());
    }

    #[test]
    fn restore_puts_the_task_back_at_the_end_of_its_column() {
        let service = service();
        let first = service.create_task(new_task("1")).unwrap();
        let second = service.create_task(new_task("2")).unwrap();

        service.delete_task(first.id).unwrap();
        // 削除中に増えたタスクが、消したタスクの古いキーを追い越しうる。
        let third = service.create_task(new_task("3")).unwrap();

        let restored = service.restore_task(first.id).unwrap();
        assert!(!restored.is_deleted());
        assert!(restored.sort_order > third.sort_order);

        let board = service.board().unwrap();
        let ids: Vec<TaskId> = board.columns.new.iter().map(|c| c.task.id).collect();
        assert_eq!(ids, [second.id, third.id, first.id]);
        assert!(service.list_deleted().unwrap().is_empty());
    }

    #[test]
    fn deleted_tasks_reject_normal_operations() {
        let service = service();
        let task = service.create_task(new_task("消す")).unwrap();
        service.delete_task(task.id).unwrap();

        fn deleted<T>(result: CoreResult<T>) -> bool {
            matches!(result, Err(CoreError::TaskDeleted(_)))
        }

        assert!(deleted(service.task_detail(task.id)));
        assert!(deleted(service.update_task(
            task.id,
            TaskPatch {
                title: Some("新しい".to_owned()),
                ..TaskPatch::default()
            }
        )));
        assert!(deleted(
            service.move_task(task.id, MoveRequest::to_column(BoardColumn::Today))
        ));
        assert!(deleted(service.complete_task(task.id)));
        assert!(deleted(service.promote_task(task.id, None)));
        assert!(deleted(service.add_task_update(
            task.id,
            "メモ",
            Origin::User
        )));
        assert!(deleted(service.add_resource(
            task.id,
            NewResource {
                kind: ResourceKind::Url,
                value: "https://example.com".to_owned(),
                label: String::new(),
                is_primary: false,
            }
        )));
        assert!(deleted(service.set_parent(task.id, None)));

        // 削除済みタスクを親にすることもできない。
        let other = service.create_task(new_task("別")).unwrap();
        assert!(deleted(service.set_parent(other.id, Some(task.id))));
    }

    #[test]
    fn delete_and_restore_are_idempotent() {
        let service = service();
        let task = service.create_task(new_task("冪等")).unwrap();

        // 生存中の復元は何もしない。
        let untouched = service.restore_task(task.id).unwrap();
        assert_eq!(untouched.sort_order, task.sort_order);
        assert!(!untouched.is_deleted());

        let first = service.delete_task(task.id).unwrap();
        let second = service.delete_task(task.id).unwrap();
        assert_eq!(first.deleted_at, second.deleted_at);

        service.restore_task(task.id).unwrap();
        let again = service.restore_task(task.id).unwrap();
        assert!(!again.is_deleted());

        // 存在しないタスクは削除も復元もできない。
        let missing = TaskId::new();
        assert!(matches!(
            service.delete_task(missing),
            Err(CoreError::TaskNotFound(_))
        ));
        assert!(matches!(
            service.restore_task(missing),
            Err(CoreError::TaskNotFound(_))
        ));
    }

    #[test]
    fn deleted_tasks_are_listed_newest_first() {
        let service = service();
        let a = service.create_task(new_task("a")).unwrap();
        let b = service.create_task(new_task("b")).unwrap();
        service.delete_task(a.id).unwrap();
        service.delete_task(b.id).unwrap();

        let deleted = service.list_deleted().unwrap();
        assert_eq!(deleted.len(), 2);
        // FixedClock なので削除時刻は同値。順序が壊れないことだけを見る。
        let ids: HashSet<TaskId> = deleted.iter().map(|c| c.task.id).collect();
        assert_eq!(ids, HashSet::from([a.id, b.id]));
        assert!(deleted.iter().all(|card| card.task.deleted_at.is_some()));
    }

    #[tokio::test]
    async fn delete_and_restore_are_broadcast() {
        let service = service();
        let mut rx = service.subscribe();
        let task = service.create_task(new_task("通知")).unwrap();
        assert_eq!(
            rx.recv().await.unwrap(),
            DomainEvent::TaskCreated { task_id: task.id }
        );

        service.delete_task(task.id).unwrap();
        assert_eq!(
            rx.recv().await.unwrap(),
            DomainEvent::TaskDeleted { task_id: task.id }
        );

        service.restore_task(task.id).unwrap();
        assert_eq!(
            rx.recv().await.unwrap(),
            DomainEvent::TaskRestored { task_id: task.id }
        );

        // 冪等な呼び出しではイベントを出さない。
        service.restore_task(task.id).unwrap();
        service.notify_settings_changed();
        assert_eq!(rx.recv().await.unwrap(), DomainEvent::SettingsChanged);
    }

    #[test]
    fn board_counts_ignore_resources_of_deleted_tasks() {
        let service = service();
        let task = service
            .create_task(NewTask {
                title: "資料".to_owned(),
                resources: vec![NewResource {
                    kind: ResourceKind::Url,
                    value: "https://example.com".to_owned(),
                    label: String::new(),
                    is_primary: true,
                }],
                ..NewTask::default()
            })
            .unwrap();
        service.delete_task(task.id).unwrap();

        let board = service.board().unwrap();
        assert!(board.columns.new.is_empty());
        // 削除済み一覧では集計しない。
        let deleted = service.list_deleted().unwrap();
        assert_eq!(deleted[0].resource_count, 0);
        assert!(deleted[0].primary_resource.is_none());
    }

    #[test]
    fn board_columns_iterate_left_to_right() {
        let service = service();
        let task = service.create_task(new_task("受信箱")).unwrap();
        service
            .move_task(task.id, MoveRequest::to_column(BoardColumn::Doing))
            .unwrap();

        let board = service.board().unwrap();
        let columns: Vec<BoardColumn> = board.columns.iter().map(|(column, _)| column).collect();
        assert_eq!(
            columns,
            [
                BoardColumn::New,
                BoardColumn::Today,
                BoardColumn::Tomorrow,
                BoardColumn::ThisWeek,
                BoardColumn::NextWeek,
                BoardColumn::Future,
                BoardColumn::Watching,
                BoardColumn::Doing,
                BoardColumn::Done,
            ]
        );
        let filled: Vec<(BoardColumn, usize)> = board
            .columns
            .iter()
            .filter(|(_, cards)| !cards.is_empty())
            .map(|(column, cards)| (column, cards.len()))
            .collect();
        assert_eq!(filled, [(BoardColumn::Doing, 1)]);
    }

    // ---- Watching(外部の変化待ち)----

    /// 予定を持ったまま Watching へ入れたタスクを返す。
    fn watching_task(service: &TaskService, title: &str) -> Task {
        let task = service.create_task(new_task(title)).unwrap();
        // 予定が保持されることを見たいので、一度 NextWeek に置いてから Watching へ移す。
        service
            .move_task(task.id, MoveRequest::to_column(BoardColumn::NextWeek))
            .unwrap();
        let watching = service
            .move_task(task.id, MoveRequest::to_column(BoardColumn::Watching))
            .unwrap();
        assert_eq!(watching.status, TaskStatus::Watching);
        assert!(
            matches!(watching.scheduled, Scheduled::Week(_)),
            "Watching へ移しても予定は保持される"
        );
        watching
    }

    #[test]
    fn move_to_watching_keeps_the_schedule_and_has_no_bucket() {
        let service = service();
        let task = watching_task(&service, "PR のレビュー待ち");

        let board = service.board().unwrap();
        assert_eq!(board.columns.watching.len(), 1);
        assert_eq!(board.columns.watching[0].task.id, task.id);
        // Watching はバケットを持たない(Todo ではないため)。
        assert_eq!(board.columns.watching[0].bucket, None);
        assert!(board.columns.next_week.is_empty());
    }

    #[test]
    fn non_user_update_wakes_a_watching_task() {
        let service = service();
        let task = watching_task(&service, "CI の結果待ち");
        let before = task.scheduled;

        service
            .add_task_update(
                task.id,
                "CI が失敗しました",
                Origin::Plugin("github".into()),
            )
            .unwrap();

        let woken = service.find_task(task.id).unwrap().expect("生きている");
        assert_eq!(woken.status, TaskStatus::New);
        assert_eq!(woken.scheduled, before, "起床しても予定は保持する");

        let board = service.board().unwrap();
        assert!(board.columns.watching.is_empty());
        assert_eq!(board.columns.new.len(), 1);
        assert_eq!(board.columns.new[0].task.id, task.id);
    }

    #[test]
    fn every_non_user_origin_wakes_a_watching_task() {
        for origin in [
            Origin::Mcp,
            Origin::Ai,
            Origin::System,
            Origin::Plugin("github".into()),
        ] {
            let service = service();
            let task = watching_task(&service, "待ち");
            service
                .add_task_update(task.id, "変化", origin.clone())
                .unwrap();
            assert_eq!(
                service.find_task(task.id).unwrap().unwrap().status,
                TaskStatus::New,
                "{origin} では起床する"
            );
        }
    }

    #[test]
    fn user_updates_do_not_wake_a_watching_task() {
        let service = service();
        let task = watching_task(&service, "自分でメモする");

        service
            .add_task_update(task.id, "あとで見る", Origin::User)
            .unwrap();

        let still = service.find_task(task.id).unwrap().expect("生きている");
        assert_eq!(still.status, TaskStatus::Watching);
        assert_eq!(service.board().unwrap().columns.watching.len(), 1);
    }

    #[test]
    fn updates_on_non_watching_tasks_change_nothing() {
        let service = service();
        let task = service.create_task(new_task("ふつうのタスク")).unwrap();
        service
            .move_task(task.id, MoveRequest::to_column(BoardColumn::Doing))
            .unwrap();

        service
            .add_task_update(task.id, "MCP から追記", Origin::Mcp)
            .unwrap();

        assert_eq!(
            service.find_task(task.id).unwrap().unwrap().status,
            TaskStatus::Doing,
            "Watching でないタスクは動かさない"
        );
    }

    #[test]
    fn waking_is_idempotent() {
        let service = service();
        let task = watching_task(&service, "何度でも");

        service
            .add_task_update(task.id, "1 回目", Origin::Mcp)
            .unwrap();
        let first = service.find_task(task.id).unwrap().unwrap();
        service
            .add_task_update(task.id, "2 回目", Origin::Mcp)
            .unwrap();
        let second = service.find_task(task.id).unwrap().unwrap();

        assert_eq!(second.status, TaskStatus::New);
        assert_eq!(
            second.sort_order, first.sort_order,
            "既に New なら並び順も動かさない"
        );
    }

    #[test]
    fn waking_appends_to_the_end_of_the_new_column() {
        let service = service();
        let watching = watching_task(&service, "起こされる");
        let a = service.create_task(new_task("先客 a")).unwrap();
        let b = service.create_task(new_task("先客 b")).unwrap();

        service
            .add_task_update(watching.id, "変化", Origin::Mcp)
            .unwrap();

        let board = service.board().unwrap();
        let ids: Vec<TaskId> = board.columns.new.iter().map(|c| c.task.id).collect();
        assert_eq!(ids, [a.id, b.id, watching.id]);
    }

    #[test]
    fn a_child_created_by_a_non_user_origin_wakes_the_watching_parent() {
        let service = service();
        let parent = watching_task(&service, "PR を見張る");

        service
            .create_task(NewTask {
                title: "PR を確認する".to_owned(),
                origin: Origin::Plugin("github".into()),
                parent_id: Some(parent.id),
                is_instant: true,
                ..NewTask::default()
            })
            .unwrap();

        assert_eq!(
            service.find_task(parent.id).unwrap().unwrap().status,
            TaskStatus::New
        );
    }

    #[test]
    fn a_child_created_by_the_user_leaves_the_watching_parent_alone() {
        let service = service();
        let parent = watching_task(&service, "見張り続ける");

        service
            .create_task(NewTask {
                title: "自分で足した子".to_owned(),
                parent_id: Some(parent.id),
                ..NewTask::default()
            })
            .unwrap();

        assert_eq!(
            service.find_task(parent.id).unwrap().unwrap().status,
            TaskStatus::Watching
        );
    }

    /// 親が Watching でなければ、外部 origin の子タスク作成でも何も起きない。
    #[test]
    fn creating_a_child_under_a_normal_parent_changes_nothing() {
        let service = service();
        let parent = service.create_task(new_task("ふつうの親")).unwrap();
        service
            .move_task(parent.id, MoveRequest::to_column(BoardColumn::Today))
            .unwrap();
        let before = service.find_task(parent.id).unwrap().unwrap();

        service
            .create_task(NewTask {
                title: "子".to_owned(),
                origin: Origin::Mcp,
                parent_id: Some(parent.id),
                ..NewTask::default()
            })
            .unwrap();

        let after = service.find_task(parent.id).unwrap().unwrap();
        assert_eq!(after.status, TaskStatus::Todo);
        assert_eq!(after.sort_order, before.sort_order);
    }

    #[tokio::test]
    async fn waking_broadcasts_woken_then_moved() {
        let service = service();
        let task = watching_task(&service, "通知");
        let mut rx = service.subscribe();

        service
            .add_task_update(task.id, "外から変化がありました", Origin::Mcp)
            .unwrap();

        assert_eq!(
            rx.recv().await.unwrap(),
            DomainEvent::TaskUpdateAdded { task_id: task.id }
        );
        assert_eq!(
            rx.recv().await.unwrap(),
            DomainEvent::TaskWoken { task_id: task.id }
        );
        assert_eq!(
            rx.recv().await.unwrap(),
            DomainEvent::TaskMoved {
                task_id: task.id,
                status: TaskStatus::New,
                bucket: None,
            }
        );
    }

    /// 起きなかったときは起床イベントを出さない。
    #[tokio::test]
    async fn a_user_update_broadcasts_no_wake() {
        let service = service();
        let task = watching_task(&service, "静かなまま");
        let mut rx = service.subscribe();

        service
            .add_task_update(task.id, "メモ", Origin::User)
            .unwrap();
        service.notify_settings_changed();

        assert_eq!(
            rx.recv().await.unwrap(),
            DomainEvent::TaskUpdateAdded { task_id: task.id }
        );
        assert_eq!(rx.recv().await.unwrap(), DomainEvent::SettingsChanged);
    }

    /// ボードの JSON 契約に `watching` 列が含まれること(フロントの `BoardColumns` と対応)。
    #[test]
    fn board_json_has_a_watching_column() {
        let service = service();
        let task = watching_task(&service, "契約");

        let board = serde_json::to_value(service.board().unwrap()).unwrap();
        let columns = board["columns"]
            .as_object()
            .expect("columns はオブジェクト");
        assert_eq!(
            columns.keys().map(String::as_str).collect::<HashSet<_>>(),
            HashSet::from([
                "new", "today", "tomorrow", "thisWeek", "nextWeek", "future", "watching", "doing",
                "done",
            ])
        );
        let card = &board["columns"]["watching"][0];
        assert_eq!(card["id"], task.id.to_string());
        assert_eq!(card["status"], "watching");
        assert!(card["bucket"].is_null());
    }

    // ---- 過去の完了(Done 列の絞り込み)----

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("有効な日付")
    }

    fn instant(raw: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(raw)
            .expect("RFC3339")
            .with_timezone(&Utc)
    }

    /// リポジトリを共有したまま、別のクロックのサービスを作る。
    ///
    /// 「昨日終えたタスク」は昨日のクロックで完了させるしかないので、
    /// 日付をまたぐテストはこれで時計を進める。
    fn service_at(repo: &Arc<MemoryRepository>, clock: FixedClock) -> TaskService {
        TaskService::new(
            Arc::clone(repo) as Arc<dyn TaskRepository>,
            Arc::new(clock),
            BoardSettings::default(),
        )
    }

    /// 指定のクロックでタスクを 1 件作り、そのまま完了させる。
    fn complete_at(repo: &Arc<MemoryRepository>, clock: FixedClock, title: &str) -> TaskId {
        let service = service_at(repo, clock);
        let task = service.create_task(new_task(title)).unwrap();
        service.complete_task(task.id).unwrap();
        task.id
    }

    fn done_ids(board: &Board) -> Vec<TaskId> {
        board.columns.done.iter().map(|card| card.task.id).collect()
    }

    #[test]
    fn the_done_column_keeps_only_todays_completions() {
        let repo = Arc::new(MemoryRepository::default());
        let today = date(2026, 9, 2);
        let old = complete_at(&repo, FixedClock::at(date(2026, 9, 1)), "昨日終えた");

        let service = service_at(&repo, FixedClock::at(today));
        let fresh = service.create_task(new_task("今日終えた")).unwrap();
        service.complete_task(fresh.id).unwrap();

        let board = service.board().unwrap();
        assert_eq!(done_ids(&board), [fresh.id], "昨日の完了は Done 列に出ない");
        assert_eq!(board.archived_done_count, 1);

        // 一覧には昨日の分だけが出る。
        let archived = service.list_archived_done().unwrap();
        assert_eq!(archived.total, 1);
        assert_eq!(archived.limit, ARCHIVED_DONE_LIMIT);
        assert_eq!(
            archived.tasks.iter().map(|c| c.task.id).collect::<Vec<_>>(),
            [old]
        );
    }

    /// MCP のように「全部見せたい」呼び出し側のための入口。
    #[test]
    fn the_full_board_keeps_every_completion() {
        let repo = Arc::new(MemoryRepository::default());
        let old = complete_at(&repo, FixedClock::at(date(2026, 9, 1)), "昨日終えた");
        let service = service_at(&repo, FixedClock::at(date(2026, 9, 2)));
        let fresh = complete_at(&repo, FixedClock::at(date(2026, 9, 2)), "今日終えた");

        let full = service.full_board().unwrap();
        let listed: HashSet<TaskId> = done_ids(&full).into_iter().collect();
        assert_eq!(listed, HashSet::from([old, fresh]));
        // 「そのうち何件が過去の完了か」の意味は board() と同じ。
        assert_eq!(full.archived_done_count, 1);
    }

    /// 日付が変わるだけで、データを動かさずに Done 列から消えること。
    #[test]
    fn advancing_the_clock_archives_todays_completions() {
        let repo = Arc::new(MemoryRepository::default());
        let done = complete_at(&repo, FixedClock::at(date(2026, 9, 2)), "今日終えた");

        let today = service_at(&repo, FixedClock::at(date(2026, 9, 2)));
        let board = today.board().unwrap();
        assert_eq!(done_ids(&board), [done]);
        assert_eq!(board.archived_done_count, 0);

        // 同じ DB のまま翌日になると、Done 列から落ちて一覧へ回る。
        let tomorrow = service_at(&repo, FixedClock::at(date(2026, 9, 3)));
        let board = tomorrow.board().unwrap();
        assert!(board.columns.done.is_empty());
        assert_eq!(board.archived_done_count, 1);
        assert_eq!(
            tomorrow.list_archived_done().unwrap().tasks[0].task.id,
            done
        );
    }

    /// UTC では前日でも、ローカル(+9)では今日なら Done 列に残る。
    #[test]
    fn the_boundary_follows_the_local_day_not_utc() {
        let repo = Arc::new(MemoryRepository::default());
        let today = date(2026, 9, 2);
        // 09-01 23:00Z = 09-02 08:00 (+09:00) → ローカルでは今日。
        let local_today = complete_at(
            &repo,
            FixedClock::new(instant("2026-09-01T23:00:00Z"), today).with_offset_hours(9),
            "ローカルでは今日",
        );
        // 09-01 14:00Z = 09-01 23:00 (+09:00) → ローカルでも昨日。
        let local_yesterday = complete_at(
            &repo,
            FixedClock::new(instant("2026-09-01T14:00:00Z"), today).with_offset_hours(9),
            "ローカルでも昨日",
        );

        let service = service_at(
            &repo,
            FixedClock::new(instant("2026-09-02T03:00:00Z"), today).with_offset_hours(9),
        );
        let board = service.board().unwrap();
        assert_eq!(done_ids(&board), [local_today]);
        assert_eq!(board.archived_done_count, 1);
        assert_eq!(
            service.list_archived_done().unwrap().tasks[0].task.id,
            local_yesterday
        );

        // 同じデータでも UTC のクロックなら、どちらも「昨日」になる。
        let utc = service_at(
            &repo,
            FixedClock::new(instant("2026-09-02T03:00:00Z"), today),
        );
        assert!(utc.board().unwrap().columns.done.is_empty());
        assert_eq!(utc.list_archived_done().unwrap().total, 2);
    }

    #[test]
    fn archived_done_is_newest_first_and_capped() {
        let repo = Arc::new(MemoryRepository::default());
        let today = date(2026, 9, 10);
        let clock_at = |raw: &str| FixedClock::new(instant(raw), today);

        let oldest = complete_at(&repo, clock_at("2026-09-01T00:00:00Z"), "いちばん古い");
        let middle = complete_at(&repo, clock_at("2026-09-05T00:00:00Z"), "まんなか");
        let newest = complete_at(&repo, clock_at("2026-09-09T23:00:00Z"), "いちばん新しい");

        let service = service_at(&repo, FixedClock::at(today));
        let archived = service.list_archived_done().unwrap();
        assert_eq!(
            archived.tasks.iter().map(|c| c.task.id).collect::<Vec<_>>(),
            [newest, middle, oldest],
            "完了が新しい順"
        );
        assert_eq!(archived.total, 3);

        // 上限を超えたら、総件数はそのままに返す件数だけが切り詰められる。
        for i in 0..ARCHIVED_DONE_LIMIT {
            complete_at(&repo, clock_at("2026-09-02T00:00:00Z"), &format!("古い{i}"));
        }
        let archived = service.list_archived_done().unwrap();
        assert_eq!(archived.total, ARCHIVED_DONE_LIMIT + 3);
        assert_eq!(archived.tasks.len(), ARCHIVED_DONE_LIMIT);
        assert_eq!(archived.tasks[0].task.id, newest, "新しい方から返る");
    }

    /// 削除したタスクは、過去の完了一覧にもボードの件数にも出ない。
    #[test]
    fn deleted_tasks_are_not_archived_done() {
        let repo = Arc::new(MemoryRepository::default());
        let old = complete_at(&repo, FixedClock::at(date(2026, 9, 1)), "昨日終えて消した");

        let service = service_at(&repo, FixedClock::at(date(2026, 9, 2)));
        service.delete_task(old).unwrap();

        assert_eq!(service.board().unwrap().archived_done_count, 0);
        let archived = service.list_archived_done().unwrap();
        assert_eq!(archived.total, 0);
        assert!(archived.tasks.is_empty());
    }

    /// `done_at` が無い完了タスク(古いデータ)は隠さない。
    #[test]
    fn a_completion_without_a_timestamp_stays_visible() {
        let repo = Arc::new(MemoryRepository::default());
        let service = service_at(&repo, FixedClock::at(date(2026, 9, 2)));
        let task = service.create_task(new_task("いつ終えたか不明")).unwrap();
        service.complete_task(task.id).unwrap();
        // サービス経由では作れない状態を、リポジトリへ直接書いて再現する。
        let mut stored = repo.find_task(task.id).unwrap().unwrap();
        stored.done_at = None;
        assert!(repo.update_task(&stored).unwrap());

        let board = service.board().unwrap();
        assert_eq!(done_ids(&board), [task.id]);
        assert_eq!(board.archived_done_count, 0);
        assert_eq!(service.list_archived_done().unwrap().total, 0);
    }
}
