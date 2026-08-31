//! ツール層のロジックを、インメモリ SQLite に載せたサービスで検証する。

use questloom_core::model::TaskStatus;
use questloom_mcp::tools::{
    AddResourceArgs, AddTaskUpdateArgs, ColumnArg, CreateTaskArgs, ListTasksArgs, MoveTaskArgs,
    PromoteTaskArgs, ResourceArg, ResourceKindArg, StatusArg, TaskIdArgs, UpdateTaskArgs,
};
use questloom_mcp::QuestloomTools;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use serde_json::Value;

mod common;

fn tools() -> QuestloomTools {
    QuestloomTools::new(common::service())
}

/// ツール結果のテキストを JSON として取り出す。エラー結果ならパニックする。
#[track_caller]
fn json(result: &CallToolResult) -> Value {
    assert_ne!(
        result.is_error,
        Some(true),
        "ツールがエラーを返しました: {result:?}"
    );
    text(result)
}

#[track_caller]
fn text(result: &CallToolResult) -> Value {
    let block = result.content.first().expect("content が 1 件以上ある");
    let text = block.as_text().expect("テキストの content").text.clone();
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

fn create(tools: &QuestloomTools, args: CreateTaskArgs) -> Value {
    json(&tools.create_task(Parameters(args)).expect("create_task"))
}

fn new_task(title: &str) -> CreateTaskArgs {
    CreateTaskArgs {
        title: title.to_owned(),
        description: None,
        column: None,
        deadline: None,
        is_instant: None,
        parent_id: None,
        resources: None,
    }
}

fn list(tools: &QuestloomTools, args: ListTasksArgs) -> Value {
    json(&tools.list_tasks(Parameters(args)).expect("list_tasks"))
}

#[test]
fn create_defaults_to_an_instant_task_in_new() {
    let tools = tools();
    let created = create(&tools, new_task("PR を見る"));
    assert_eq!(created["status"], "new");
    assert_eq!(created["column"], "new");
    assert_eq!(created["isInstant"], true);
    assert_eq!(created["origin"], "mcp");
    assert!(created["bucket"].is_null());
}

#[test]
fn create_with_a_column_makes_a_regular_task() {
    let tools = tools();
    let created = create(
        &tools,
        CreateTaskArgs {
            column: Some(ColumnArg::Tomorrow),
            deadline: Some("2026-09-30T09:00:00Z".to_owned()),
            description: Some("詳細".to_owned()),
            ..new_task("資料を作る")
        },
    );
    assert_eq!(created["status"], "todo");
    assert_eq!(created["column"], "tomorrow");
    assert_eq!(created["bucket"], "tomorrow");
    assert_eq!(created["isInstant"], false);
    assert_eq!(created["scheduled"]["kind"], "date");
    assert_eq!(created["deadline"], "2026-09-30T09:00:00Z");
}

/// create → list → move → complete の一連が通ることを確認する。
#[test]
fn create_list_move_complete_round_trip() {
    let tools = tools();
    let created = create(&tools, new_task("片付ける"));
    let task_id = created["id"].as_str().expect("id は文字列").to_owned();

    let listed = list(&tools, ListTasksArgs::default());
    assert_eq!(listed["count"], 1);
    assert_eq!(listed["tasks"][0]["id"], task_id.as_str());

    // 列で絞り込める。
    let listed = list(
        &tools,
        ListTasksArgs {
            column: Some(ColumnArg::New),
            status: None,
        },
    );
    assert_eq!(listed["count"], 1);
    let listed = list(
        &tools,
        ListTasksArgs {
            column: Some(ColumnArg::Today),
            status: None,
        },
    );
    assert_eq!(listed["count"], 0);

    let moved = json(
        &tools
            .move_task(Parameters(MoveTaskArgs {
                task_id: task_id.clone(),
                column: ColumnArg::Today,
            }))
            .expect("move_task"),
    );
    assert_eq!(moved["status"], "todo");
    assert_eq!(moved["bucket"], "today");

    // ステータスで絞り込める。
    let listed = list(
        &tools,
        ListTasksArgs {
            status: Some(StatusArg::Todo),
            column: None,
        },
    );
    assert_eq!(listed["count"], 1);
    assert_eq!(listed["tasks"][0]["column"], "today");

    let done = json(
        &tools
            .complete_task(Parameters(TaskIdArgs {
                task_id: task_id.clone(),
            }))
            .expect("complete_task"),
    );
    assert_eq!(done["status"], "done");
    assert_eq!(done["column"], "done");

    let listed = list(
        &tools,
        ListTasksArgs {
            status: Some(StatusArg::Done),
            column: None,
        },
    );
    assert_eq!(listed["count"], 1);

    // サービス側にも反映されている。
    let tasks = tools
        .service()
        .list_by_status(TaskStatus::Done)
        .expect("list_by_status");
    assert_eq!(tasks.len(), 1);
}

#[test]
fn update_get_and_history() {
    let tools = tools();
    let created = create(
        &tools,
        CreateTaskArgs {
            deadline: Some("2026-09-30T09:00:00Z".to_owned()),
            resources: Some(vec![ResourceArg {
                kind: ResourceKindArg::Url,
                value: "https://example.com/pr/1".to_owned(),
                label: None,
                is_primary: None,
            }]),
            ..new_task("調べる")
        },
    );
    let task_id = created["id"].as_str().unwrap().to_owned();

    let updated = json(
        &tools
            .update_task(Parameters(UpdateTaskArgs {
                task_id: task_id.clone(),
                title: Some("よく調べる".to_owned()),
                description: Some("詳細を追記".to_owned()),
                deadline: None,
                clear_deadline: Some(true),
            }))
            .expect("update_task"),
    );
    assert_eq!(updated["title"], "よく調べる");
    assert!(updated["deadline"].is_null());

    json(
        &tools
            .add_task_update(Parameters(AddTaskUpdateArgs {
                task_id: task_id.clone(),
                body: "半分終わった".to_owned(),
            }))
            .expect("add_task_update"),
    );
    json(
        &tools
            .add_resource(Parameters(AddResourceArgs {
                task_id: task_id.clone(),
                kind: ResourceKindArg::File,
                value: "C:/tmp/memo.txt".to_owned(),
                label: Some("メモ".to_owned()),
                is_primary: Some(true),
            }))
            .expect("add_resource"),
    );

    let detail = json(
        &tools
            .get_task(Parameters(TaskIdArgs {
                task_id: task_id.clone(),
            }))
            .expect("get_task"),
    );
    assert_eq!(detail["id"], task_id.as_str());
    assert_eq!(detail["description"], "詳細を追記");
    assert_eq!(detail["resources"].as_array().unwrap().len(), 2);
    assert_eq!(detail["updates"].as_array().unwrap().len(), 1);
    assert_eq!(detail["updates"][0]["origin"], "mcp");
    assert_eq!(detail["primaryResource"]["value"], "C:/tmp/memo.txt");
}

#[test]
fn promote_turns_an_instant_task_into_a_regular_one() {
    let tools = tools();
    let created = create(&tools, new_task("あとでやる"));
    let task_id = created["id"].as_str().unwrap().to_owned();

    let promoted = json(
        &tools
            .promote_task(Parameters(PromoteTaskArgs {
                task_id: task_id.clone(),
                column: Some(ColumnArg::NextWeek),
            }))
            .expect("promote_task"),
    );
    assert_eq!(promoted["isInstant"], false);
    assert_eq!(promoted["column"], "nextWeek");

    // 2 回目はインスタントタスクでないのでツールレベルのエラーになる。
    let again = tools
        .promote_task(Parameters(PromoteTaskArgs {
            task_id,
            column: None,
        }))
        .expect("promote_task は Err を返さない");
    assert_eq!(again.is_error, Some(true));
}

#[test]
fn parent_child_links_are_supported() {
    let tools = tools();
    let parent = create(&tools, new_task("親"));
    let parent_id = parent["id"].as_str().unwrap().to_owned();
    let child = create(
        &tools,
        CreateTaskArgs {
            parent_id: Some(parent_id.clone()),
            ..new_task("子")
        },
    );
    assert_eq!(child["parentId"], parent_id.as_str());

    let detail = json(
        &tools
            .get_task(Parameters(TaskIdArgs { task_id: parent_id }))
            .expect("get_task"),
    );
    assert_eq!(detail["children"].as_array().unwrap().len(), 1);
}

/// delete → 一覧から消える → restore → 戻る、の一連。
#[test]
fn delete_hides_a_task_until_it_is_restored() {
    let tools = tools();
    let created = create(
        &tools,
        CreateTaskArgs {
            column: Some(ColumnArg::Today),
            ..new_task("消して戻す")
        },
    );
    let task_id = created["id"].as_str().unwrap().to_owned();
    let keep = create(&tools, new_task("残す"));
    assert_eq!(list(&tools, ListTasksArgs::default())["count"], 2);

    let deleted = json(
        &tools
            .delete_task(Parameters(TaskIdArgs {
                task_id: task_id.clone(),
            }))
            .expect("delete_task"),
    );
    assert_eq!(deleted["deleted"], true);
    assert!(!deleted["deletedAt"].is_null());

    // 一覧からは消え、詳細も引けなくなる。
    let listed = list(&tools, ListTasksArgs::default());
    assert_eq!(listed["count"], 1);
    assert_eq!(listed["tasks"][0]["id"], keep["id"]);
    let gone = tools
        .get_task(Parameters(TaskIdArgs {
            task_id: task_id.clone(),
        }))
        .expect("Err にはならない");
    assert_eq!(gone.is_error, Some(true));

    // 削除済みへの通常操作もツールレベルのエラー。
    let rejected = tools
        .complete_task(Parameters(TaskIdArgs {
            task_id: task_id.clone(),
        }))
        .expect("Err にはならない");
    assert_eq!(rejected.is_error, Some(true));

    // 2 回目の削除は冪等に成功し、削除済みが増えたりはしない。
    let again = json(
        &tools
            .delete_task(Parameters(TaskIdArgs {
                task_id: task_id.clone(),
            }))
            .expect("delete_task"),
    );
    assert_eq!(again["deleted"], true);
    let deleted_cards = tools.service().list_deleted().expect("list_deleted");
    assert_eq!(deleted_cards.len(), 1);
    assert_eq!(deleted_cards[0].task.title, "消して戻す");

    // 復元すると元の列へ戻る。
    let restored = json(
        &tools
            .restore_task(Parameters(TaskIdArgs {
                task_id: task_id.clone(),
            }))
            .expect("restore_task"),
    );
    assert_eq!(restored["column"], "today");
    assert_eq!(restored["status"], "todo");

    let listed = list(&tools, ListTasksArgs::default());
    assert_eq!(listed["count"], 2);
    assert_eq!(
        json(
            &tools
                .get_task(Parameters(TaskIdArgs { task_id }))
                .expect("get_task")
        )["title"],
        "消して戻す"
    );
}

#[test]
fn invalid_arguments_are_reported() {
    let tools = tools();

    // UUID として解釈できない ID はプロトコルエラー。
    let error = tools
        .get_task(Parameters(TaskIdArgs {
            task_id: "not-a-uuid".to_owned(),
        }))
        .expect_err("invalid_params になる");
    assert!(error.message.contains("invalid task_id"), "{error:?}");

    // RFC 3339 でない締切も同様。
    let error = tools
        .create_task(Parameters(CreateTaskArgs {
            deadline: Some("明日".to_owned()),
            ..new_task("締切")
        }))
        .expect_err("invalid_params になる");
    assert!(error.message.contains("invalid deadline"), "{error:?}");

    // 存在しないタスクはツールレベルのエラー(呼び出し側に見せる)。
    let missing = tools
        .get_task(Parameters(TaskIdArgs {
            task_id: questloom_core::model::TaskId::new().to_string(),
        }))
        .expect("Err にはならない");
    assert_eq!(missing.is_error, Some(true));

    // 空タイトルもツールレベルのエラー。
    let blank = tools
        .create_task(Parameters(new_task("   ")))
        .expect("Err にはならない");
    assert_eq!(blank.is_error, Some(true));
}
