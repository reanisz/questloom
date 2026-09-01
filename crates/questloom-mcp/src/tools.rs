//! MCP のツールセット。[`TaskService`] のユースケースをそのまま公開する。
//!
//! ツール名は snake_case、引数のフィールド名も snake_case。
//! 返り値の JSON は questloom-core の serde 表現(camelCase・週/日付は文字列)に従う。

use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use questloom_core::bucket::{bucket_for, BoardColumn, Bucket};
use questloom_core::error::{CoreError, CoreResult};
use questloom_core::model::{
    ChecklistItemId, Origin, ResourceKind, Scheduled, Task, TaskId, TaskStatus,
};
use questloom_core::service::{MoveRequest, NewResource, NewTask, TaskPatch, TaskService};
use questloom_core::settings::WeekStart;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, ErrorData, Implementation, ServerCapabilities, ServerInfo,
};
use rmcp::schemars;
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use serde::Deserialize;
use serde_json::{json, Value};

/// MCP クライアントへ提示する使い方の説明。
const INSTRUCTIONS: &str = "questloom task board. Tasks live in one of the columns \
new / today / tomorrow / thisWeek / nextWeek / future / watching / doing / done. \
Time buckets are derived from the schedule, so moving a task to a column sets its schedule. \
Tasks created here default to instant tasks in the New column, which show up in the user's \
overlay for one-click completion; pass `column` to create a regular task instead. \
The `watching` column parks a task that is waiting on something external: any change you \
make from here (add_task_update, add_checklist_item / set_checklist_item, or create_task \
with that task as `parent_id`) wakes it back up into New so the user sees it. \
Tasks also carry an in-task checklist for steps too small to be child tasks; ticking every \
item does not complete the task. \
The user's board only shows completions from today in `done` (older ones move to a separate \
\"past completions\" list), but the tools here always see every completed task.";

// ---- 引数に使う列挙型 ----
//
// questloom-core の型を schemars 依存で汚さないよう、MCP 側にミラーを持つ。

/// `status` 引数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum StatusArg {
    /// 受信箱。
    New,
    /// 着手予定。
    Todo,
    /// 着手中。
    Doing,
    /// 完了。
    Done,
    /// 外部の変化待ち。
    Watching,
}

impl From<StatusArg> for TaskStatus {
    fn from(value: StatusArg) -> Self {
        match value {
            StatusArg::New => Self::New,
            StatusArg::Todo => Self::Todo,
            StatusArg::Doing => Self::Doing,
            StatusArg::Done => Self::Done,
            StatusArg::Watching => Self::Watching,
        }
    }
}

/// `column` 引数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ColumnArg {
    /// 受信箱。
    New,
    /// 今日やる。
    Today,
    /// 明日やる。
    Tomorrow,
    /// 今週やる。
    ThisWeek,
    /// 来週やる。
    NextWeek,
    /// いつかやる。
    Future,
    /// 外部の変化待ち。
    Watching,
    /// 着手中。
    Doing,
    /// 完了。
    Done,
}

impl From<ColumnArg> for BoardColumn {
    fn from(value: ColumnArg) -> Self {
        match value {
            ColumnArg::New => Self::New,
            ColumnArg::Today => Self::Today,
            ColumnArg::Tomorrow => Self::Tomorrow,
            ColumnArg::ThisWeek => Self::ThisWeek,
            ColumnArg::NextWeek => Self::NextWeek,
            ColumnArg::Future => Self::Future,
            ColumnArg::Watching => Self::Watching,
            ColumnArg::Doing => Self::Doing,
            ColumnArg::Done => Self::Done,
        }
    }
}

/// `kind` 引数(関連リソースの種別)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ResourceKindArg {
    /// URL。
    Url,
    /// ローカルファイルパス。
    File,
}

impl From<ResourceKindArg> for ResourceKind {
    fn from(value: ResourceKindArg) -> Self {
        match value {
            ResourceKindArg::Url => Self::Url,
            ResourceKindArg::File => Self::File,
        }
    }
}

/// `create_task` に渡す関連リソース。
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ResourceArg {
    /// Resource kind: "url" or "file".
    pub kind: ResourceKindArg,
    /// The URL or the local file path.
    pub value: String,
    /// Display label. Defaults to empty.
    #[serde(default)]
    pub label: Option<String>,
    /// Make this the primary resource (the one the overlay opens with one click).
    #[serde(default)]
    pub is_primary: Option<bool>,
}

impl From<ResourceArg> for NewResource {
    fn from(value: ResourceArg) -> Self {
        Self {
            kind: value.kind.into(),
            value: value.value,
            label: value.label.unwrap_or_default(),
            is_primary: value.is_primary.unwrap_or(false),
        }
    }
}

// ---- 各ツールの引数 ----

/// `list_tasks` の引数。
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
pub struct ListTasksArgs {
    /// Only return tasks in this status ("new", "todo", "doing", "done", "watching").
    #[serde(default)]
    pub status: Option<StatusArg>,
    /// Only return tasks in this board column.
    #[serde(default)]
    pub column: Option<ColumnArg>,
}

/// タスク 1 件を指定するだけの引数。
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct TaskIdArgs {
    /// The task id (UUID).
    pub task_id: String,
}

/// `create_task` の引数。
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct CreateTaskArgs {
    /// Task title. Required, must not be blank.
    pub title: String,
    /// Task details in Markdown.
    #[serde(default)]
    pub description: Option<String>,
    /// Board column to place the task in. When omitted the task goes to New as an instant task.
    #[serde(default)]
    pub column: Option<ColumnArg>,
    /// Deadline as an RFC 3339 timestamp, e.g. "2026-09-30T09:00:00Z".
    #[serde(default)]
    pub deadline: Option<String>,
    /// Instant task flag. Defaults to true unless `column` is given.
    #[serde(default)]
    pub is_instant: Option<bool>,
    /// Parent task id (UUID) to attach this task to.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Related resources (URLs or file paths) to attach on creation.
    #[serde(default)]
    pub resources: Option<Vec<ResourceArg>>,
}

/// `update_task` の引数。
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct UpdateTaskArgs {
    /// The task id (UUID).
    pub task_id: String,
    /// New title.
    #[serde(default)]
    pub title: Option<String>,
    /// New details in Markdown.
    #[serde(default)]
    pub description: Option<String>,
    /// New deadline as an RFC 3339 timestamp.
    #[serde(default)]
    pub deadline: Option<String>,
    /// Remove the deadline. Takes precedence over `deadline`.
    #[serde(default)]
    pub clear_deadline: Option<bool>,
}

/// `move_task` の引数。
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct MoveTaskArgs {
    /// The task id (UUID).
    pub task_id: String,
    /// Destination column. The task is appended to the end of it.
    pub column: ColumnArg,
}

/// `promote_task` の引数。
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct PromoteTaskArgs {
    /// The task id (UUID). Must be an instant task.
    pub task_id: String,
    /// Destination column. Defaults to "today".
    #[serde(default)]
    pub column: Option<ColumnArg>,
}

/// `add_task_update` の引数。
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct AddTaskUpdateArgs {
    /// The task id (UUID).
    pub task_id: String,
    /// The progress note, in Markdown.
    pub body: String,
}

/// `add_resource` の引数。
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct AddResourceArgs {
    /// The task id (UUID).
    pub task_id: String,
    /// Resource kind: "url" or "file".
    pub kind: ResourceKindArg,
    /// The URL or the local file path.
    pub value: String,
    /// Display label.
    #[serde(default)]
    pub label: Option<String>,
    /// Make this the primary resource. The first resource always becomes primary.
    #[serde(default)]
    pub is_primary: Option<bool>,
}

/// `add_checklist_item` の引数。
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct AddChecklistItemArgs {
    /// The task id (UUID).
    pub task_id: String,
    /// The checklist item text. Required, must not be blank.
    pub body: String,
}

/// `set_checklist_item` の引数。
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct SetChecklistItemArgs {
    /// The task id (UUID).
    pub task_id: String,
    /// The checklist item id (UUID), from `get_task`.
    pub item_id: String,
    /// Tick or untick the item.
    #[serde(default)]
    pub checked: Option<bool>,
    /// Replace the item text. Must not be blank.
    #[serde(default)]
    pub body: Option<String>,
}

/// `remove_checklist_item` の引数。
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RemoveChecklistItemArgs {
    /// The task id (UUID).
    pub task_id: String,
    /// The checklist item id (UUID), from `get_task`.
    pub item_id: String,
}

// ---- ツール本体 ----

/// [`TaskService`] を MCP ツールとして公開するハンドラ。
///
/// [`StreamableHttpService`](rmcp::transport::streamable_http_server::StreamableHttpService)
/// はセッションごとにハンドラを生成するため、`Clone` は `Arc` の複製で済む。
#[derive(Clone)]
pub struct QuestloomTools {
    service: Arc<TaskService>,
    tool_router: ToolRouter<Self>,
}

impl std::fmt::Debug for QuestloomTools {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuestloomTools").finish_non_exhaustive()
    }
}

#[tool_router]
impl QuestloomTools {
    /// サービスを包むハンドラを作る。
    #[must_use]
    pub fn new(service: Arc<TaskService>) -> Self {
        Self {
            service,
            tool_router: Self::tool_router(),
        }
    }

    /// 包んでいるサービスへの参照。
    #[must_use]
    pub fn service(&self) -> &Arc<TaskService> {
        &self.service
    }

    #[tool(
        description = "List tasks on the board. Optionally filter by status and/or board column. \
             The \"done\" column here holds every completed task; the desktop board only shows \
             the ones completed today and keeps older completions in a separate list."
    )]
    pub fn list_tasks(
        &self,
        Parameters(args): Parameters<ListTasksArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(self.collect_tasks(&args))
    }

    #[tool(
        description = "Get one task in full: details, related resources, update history, parent and children."
    )]
    pub fn get_task(
        &self,
        Parameters(args): Parameters<TaskIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = parse_task_id(&args.task_id)?;
        respond(self.service.task_detail(id))
    }

    #[tool(
        description = "Create a task. By default it becomes an instant task in the New column \
                       (shown in the user's overlay). Passing `column` creates a regular task there instead."
    )]
    pub fn create_task(
        &self,
        Parameters(args): Parameters<CreateTaskArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let parent_id = args.parent_id.as_deref().map(parse_task_id).transpose()?;
        let deadline = args.deadline.as_deref().map(parse_deadline).transpose()?;
        // 列を指定しない AI 起点のタスクはインスタントタスクとして New に入る。
        let is_instant = args.is_instant.unwrap_or(args.column.is_none());
        let (status, scheduled) = match args.column {
            Some(column) => {
                let (today, week_start) = self.today_and_week_start();
                BoardColumn::from(column).resolve(Scheduled::None, today, week_start)
            }
            None => (TaskStatus::New, Scheduled::None),
        };

        let input = NewTask {
            title: args.title,
            description: args.description.unwrap_or_default(),
            status: Some(status),
            scheduled,
            deadline,
            is_instant,
            origin: Origin::Mcp,
            parent_id,
            resources: args
                .resources
                .unwrap_or_default()
                .into_iter()
                .map(NewResource::from)
                .collect(),
        };
        respond(
            self.service
                .create_task(input)
                .map(|task| self.task_summary(&task)),
        )
    }

    #[tool(
        description = "Update a task's title, details, or deadline. Omitted fields are left as-is."
    )]
    pub fn update_task(
        &self,
        Parameters(args): Parameters<UpdateTaskArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = parse_task_id(&args.task_id)?;
        let deadline = args.deadline.as_deref().map(parse_deadline).transpose()?;
        let patch = TaskPatch {
            title: args.title,
            description: args.description,
            deadline,
            clear_deadline: args.clear_deadline.unwrap_or(false),
            scheduled: None,
            is_instant: None,
        };
        respond(
            self.service
                .update_task(id, patch)
                .map(|task| self.task_summary(&task)),
        )
    }

    #[tool(
        description = "Move a task to the end of a board column. Time-bucket columns also set the \
                       task's schedule. Use \"watching\" to park a task that is waiting on something \
                       external; it wakes back into New on the next non-user change."
    )]
    pub fn move_task(
        &self,
        Parameters(args): Parameters<MoveTaskArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = parse_task_id(&args.task_id)?;
        let request = MoveRequest::to_column(args.column.into());
        respond(
            self.service
                .move_task(id, request)
                .map(|task| self.task_summary(&task)),
        )
    }

    #[tool(description = "Mark a task as done. Idempotent.")]
    pub fn complete_task(
        &self,
        Parameters(args): Parameters<TaskIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = parse_task_id(&args.task_id)?;
        respond(
            self.service
                .complete_task(id)
                .map(|task| self.task_summary(&task)),
        )
    }

    #[tool(
        description = "Promote an instant task into a regular task placed in `column` (default \"today\")."
    )]
    pub fn promote_task(
        &self,
        Parameters(args): Parameters<PromoteTaskArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = parse_task_id(&args.task_id)?;
        respond(
            self.service
                .promote_task(id, args.column.map(Into::into))
                .map(|task| self.task_summary(&task)),
        )
    }

    #[tool(
        description = "Delete a task (soft delete: it disappears from the board but can be restored \
                       with restore_task). Child tasks are not deleted. Idempotent."
    )]
    pub fn delete_task(
        &self,
        Parameters(args): Parameters<TaskIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = parse_task_id(&args.task_id)?;
        respond(self.service.delete_task(id).map(|task| {
            json!({
                "id": task.id,
                "title": task.title,
                "deleted": true,
                "deletedAt": task.deleted_at,
            })
        }))
    }

    #[tool(
        description = "Restore a deleted task. It returns to the end of its previous column. Idempotent."
    )]
    pub fn restore_task(
        &self,
        Parameters(args): Parameters<TaskIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = parse_task_id(&args.task_id)?;
        respond(
            self.service
                .restore_task(id)
                .map(|task| self.task_summary(&task)),
        )
    }

    #[tool(
        description = "Append a progress note to a task's update history. A task in the \
                       \"watching\" column wakes back into New when this is called."
    )]
    pub fn add_task_update(
        &self,
        Parameters(args): Parameters<AddTaskUpdateArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = parse_task_id(&args.task_id)?;
        respond(self.service.add_task_update(id, args.body, Origin::Mcp))
    }

    #[tool(
        description = "Append an item to a task's in-task checklist (for steps too small to be \
                       child tasks). Ticking every item does NOT complete the task. A task in the \
                       \"watching\" column wakes back into New when this is called."
    )]
    pub fn add_checklist_item(
        &self,
        Parameters(args): Parameters<AddChecklistItemArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = parse_task_id(&args.task_id)?;
        respond(self.service.add_checklist_item(id, args.body, Origin::Mcp))
    }

    #[tool(
        description = "Tick/untick or rewrite one checklist item. Item ids come from `get_task`. \
                       Omitted fields are left as-is. A task in the \"watching\" column wakes back \
                       into New when this is called."
    )]
    pub fn set_checklist_item(
        &self,
        Parameters(args): Parameters<SetChecklistItemArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = parse_task_id(&args.task_id)?;
        let item_id = parse_checklist_item_id(&args.item_id)?;
        respond(self.service.update_checklist_item(
            id,
            item_id,
            args.body,
            args.checked,
            Origin::Mcp,
        ))
    }

    #[tool(
        description = "Remove one checklist item from a task. Item ids come from `get_task`. \
                       Unlike adding or ticking, this never wakes a \"watching\" task."
    )]
    pub fn remove_checklist_item(
        &self,
        Parameters(args): Parameters<RemoveChecklistItemArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = parse_task_id(&args.task_id)?;
        let item_id = parse_checklist_item_id(&args.item_id)?;
        respond(
            self.service
                .remove_checklist_item(id, item_id, Origin::Mcp)
                .map(|()| json!({ "taskId": id, "itemId": item_id, "removed": true })),
        )
    }

    #[tool(description = "Attach a related resource (URL or local file path) to a task.")]
    pub fn add_resource(
        &self,
        Parameters(args): Parameters<AddResourceArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = parse_task_id(&args.task_id)?;
        let input = NewResource {
            kind: args.kind.into(),
            value: args.value,
            label: args.label.unwrap_or_default(),
            is_primary: args.is_primary.unwrap_or(false),
        };
        respond(self.service.add_resource(id, input))
    }

    /// ボードを集計し、絞り込みを適用した一覧 JSON を作る。
    ///
    /// 使うのは [`TaskService::full_board`] の方。UI のボードは Done 列に
    /// 「今日完了した分」しか出さないが、AI には完了済みも全部見せる。
    fn collect_tasks(&self, args: &ListTasksArgs) -> CoreResult<Value> {
        let board = self.service.full_board()?;
        let wanted = args.status.map(TaskStatus::from);
        let requested = args.column.map(BoardColumn::from);

        let mut tasks = Vec::new();
        for (column, cards) in board.columns.iter() {
            if requested.is_some_and(|wanted| wanted != column) {
                continue;
            }
            for card in cards {
                if wanted.is_some_and(|status| status != card.task.status) {
                    continue;
                }
                tasks.push(summary(column, &card.task, card.bucket));
            }
        }

        Ok(json!({
            "today": board.today,
            "weekStart": board.week_start,
            "count": tasks.len(),
            "tasks": tasks,
        }))
    }

    fn today_and_week_start(&self) -> (chrono::NaiveDate, WeekStart) {
        (self.service.today(), self.service.week_start())
    }

    /// 作成・更新直後の [`Task`] を一覧と同じ形の JSON にする。
    fn task_summary(&self, task: &Task) -> Value {
        let (today, week_start) = self.today_and_week_start();
        let bucket = bucket_for(task, today, week_start);
        summary(BoardColumn::of(task.status, bucket), task, bucket)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for QuestloomTools {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            // 既定は rmcp 自身の名前になってしまうので、questloom として名乗る。
            .with_server_info(
                Implementation::new("questloom", env!("CARGO_PKG_VERSION")).with_title("questloom"),
            )
            .with_instructions(INSTRUCTIONS)
    }
}

// ---- ヘルパ ----

fn parse_task_id(raw: &str) -> Result<TaskId, ErrorData> {
    TaskId::from_str(raw).map_err(|error| {
        ErrorData::invalid_params(format!("invalid task_id {raw:?}: {error}"), None)
    })
}

fn parse_checklist_item_id(raw: &str) -> Result<ChecklistItemId, ErrorData> {
    ChecklistItemId::from_str(raw).map_err(|error| {
        ErrorData::invalid_params(format!("invalid item_id {raw:?}: {error}"), None)
    })
}

fn parse_deadline(raw: &str) -> Result<DateTime<Utc>, ErrorData> {
    DateTime::parse_from_rfc3339(raw)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| {
            ErrorData::invalid_params(
                format!("invalid deadline {raw:?} (expected RFC 3339): {error}"),
                None,
            )
        })
}

/// サービスの実行結果をツールの応答へ変換する。
///
/// ドメインエラーは「ツールは動いたが失敗した」なので、プロトコルエラー
/// ([`ErrorData`])ではなくツールレベルのエラーとして返す。
fn respond<T: serde::Serialize>(result: CoreResult<T>) -> Result<CallToolResult, ErrorData> {
    Ok(match result {
        Ok(value) => json_result(&value),
        Err(error) => core_error(&error),
    })
}

/// ドメインエラーをツールレベルのエラーとして返す。
fn core_error(error: &CoreError) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(error.to_string())])
}

/// 値を整形した JSON テキストとして返す。JSON 化に失敗したらエラー結果にする。
fn json_result<T: serde::Serialize>(value: &T) -> CallToolResult {
    match serde_json::to_string_pretty(value) {
        Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
        Err(error) => {
            tracing::error!(%error, "ツールの応答を JSON にできませんでした");
            CallToolResult::error(vec![ContentBlock::text(format!(
                "failed to serialize the result: {error}"
            ))])
        }
    }
}

fn summary(column: BoardColumn, task: &Task, bucket: Option<Bucket>) -> Value {
    json!({
        "id": task.id,
        "title": task.title,
        "status": task.status,
        "column": column,
        "bucket": bucket,
        "isInstant": task.is_instant,
        "deadline": task.deadline,
        "scheduled": task.scheduled,
        "parentId": task.parent_id,
        "origin": task.origin,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 引数の列挙型は core 型の serde 表現と 1 対 1 でなければならない
    /// (MCP クライアントが受け取る JSON と、送れる引数を一致させるため)。
    #[test]
    fn argument_enums_round_trip_through_the_core_serde_representation() {
        for status in [
            TaskStatus::New,
            TaskStatus::Todo,
            TaskStatus::Doing,
            TaskStatus::Done,
            TaskStatus::Watching,
        ] {
            let json = serde_json::to_value(status).expect("core 型は JSON 化できる");
            let arg: StatusArg =
                serde_json::from_value(json.clone()).unwrap_or_else(|e| panic!("{json}: {e}"));
            assert_eq!(TaskStatus::from(arg), status);
        }

        for column in [
            BoardColumn::New,
            BoardColumn::Today,
            BoardColumn::Tomorrow,
            BoardColumn::ThisWeek,
            BoardColumn::NextWeek,
            BoardColumn::Future,
            BoardColumn::Watching,
            BoardColumn::Doing,
            BoardColumn::Done,
        ] {
            let json = serde_json::to_value(column).expect("core 型は JSON 化できる");
            let arg: ColumnArg =
                serde_json::from_value(json.clone()).unwrap_or_else(|e| panic!("{json}: {e}"));
            assert_eq!(BoardColumn::from(arg), column);
        }

        for kind in [ResourceKind::Url, ResourceKind::File] {
            let json = serde_json::to_value(kind).expect("core 型は JSON 化できる");
            let arg: ResourceKindArg =
                serde_json::from_value(json.clone()).unwrap_or_else(|e| panic!("{json}: {e}"));
            assert_eq!(ResourceKind::from(arg), kind);
        }

        // ドキュメント記載の綴り(camelCase)であることも固定する。
        assert_eq!(
            serde_json::to_value(BoardColumn::ThisWeek).unwrap(),
            serde_json::json!("thisWeek")
        );
    }
}
