//! 一時ディレクトリの実ファイル DB を使った統合テスト。

use std::sync::Arc;

use chrono::{NaiveDate, Utc};
use questloom_core::bucket::{BoardColumn, Bucket};
use questloom_core::clock::FixedClock;
use questloom_core::model::{Origin, ResourceKind, Scheduled, TaskStatus};
use questloom_core::repository::TaskRepository;
use questloom_core::service::{MoveRequest, NewResource, NewTask, TaskPatch, TaskService};
use questloom_core::settings::{BoardSettings, CoreSettings, WeekStart, CORE_NAMESPACE};
use questloom_store::{backup, SqliteStore, CURRENT_SCHEMA_VERSION};
use tempfile::TempDir;

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 2).expect("有効な日付")
}

fn open(dir: &TempDir) -> Arc<SqliteStore> {
    Arc::new(SqliteStore::open(dir.path().join("data.db")).expect("DB を開ける"))
}

fn make_service(store: Arc<SqliteStore>) -> TaskService {
    TaskService::new(
        store,
        Arc::new(FixedClock::at(today())),
        BoardSettings::default(),
    )
}

#[test]
fn schema_is_migrated_on_open_and_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = open(&dir);
        assert_eq!(store.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
    }
    let store = open(&dir);
    assert_eq!(store.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
    assert!(dir.path().join("data.db").exists());
}

#[test]
fn wal_mode_is_enabled_for_file_databases() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let service = make_service(Arc::clone(&store));
    service
        .create_task(NewTask {
            title: "WAL 確認".to_owned(),
            ..NewTask::default()
        })
        .unwrap();
    // WAL が有効なら書き込み時に -wal ファイルが作られる。
    assert!(dir.path().join("data.db-wal").exists());

    // チェックポイント後も内容は読める。
    store.checkpoint().unwrap();
    assert_eq!(store.list_tasks().unwrap().len(), 1);
}

#[test]
fn task_crud_roundtrips_through_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let service = make_service(Arc::clone(&store));

    let created = service
        .create_task(NewTask {
            title: "設計を書く".to_owned(),
            description: "questloom の設計".to_owned(),
            deadline: Some(Utc::now()),
            is_instant: false,
            origin: Origin::Plugin("github".to_owned()),
            ..NewTask::default()
        })
        .unwrap();

    let fetched = store
        .find_task(created.id)
        .unwrap()
        .expect("保存されている");
    assert_eq!(fetched.title, created.title);
    assert_eq!(fetched.description, created.description);
    assert_eq!(fetched.origin, Origin::Plugin("github".to_owned()));
    assert_eq!(fetched.status, TaskStatus::New);
    assert_eq!(fetched.scheduled, Scheduled::None);
    assert_eq!(
        fetched.deadline.map(|d| d.timestamp_millis()),
        created.deadline.map(|d| d.timestamp_millis())
    );

    let updated = service
        .update_task(
            created.id,
            TaskPatch {
                title: Some("設計を書き直す".to_owned()),
                clear_deadline: true,
                ..TaskPatch::default()
            },
        )
        .unwrap();
    assert_eq!(updated.title, "設計を書き直す");
    let fetched = store.find_task(created.id).unwrap().unwrap();
    assert_eq!(fetched.title, "設計を書き直す");
    assert!(fetched.deadline.is_none());

    // 更新は既存行を置き換えるだけで増えない。
    assert_eq!(store.list_tasks().unwrap().len(), 1);
}

#[test]
fn board_and_schedules_persist_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let ids = {
        let service = make_service(open(&dir));
        let a = service
            .create_task(NewTask {
                title: "今日やる".to_owned(),
                ..NewTask::default()
            })
            .unwrap();
        let b = service
            .create_task(NewTask {
                title: "来週やる".to_owned(),
                ..NewTask::default()
            })
            .unwrap();
        service
            .move_task(a.id, MoveRequest::to_column(BoardColumn::Today))
            .unwrap();
        service
            .move_task(b.id, MoveRequest::to_column(BoardColumn::NextWeek))
            .unwrap();
        (a.id, b.id)
    };

    let service = make_service(open(&dir));
    let board = service.board().unwrap();
    assert_eq!(board.columns.today.len(), 1);
    assert_eq!(board.columns.today[0].task.id, ids.0);
    assert_eq!(board.columns.today[0].bucket, Some(Bucket::Today));
    assert_eq!(board.columns.next_week.len(), 1);
    assert_eq!(board.columns.next_week[0].task.id, ids.1);
    assert_eq!(board.columns.next_week[0].bucket, Some(Bucket::NextWeek));
}

/// ソフトデリートと復元が、再オープンをまたいで残ること。
#[test]
fn soft_delete_and_restore_persist_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let (kept, dropped) = {
        let service = make_service(open(&dir));
        let kept = service
            .create_task(NewTask {
                title: "残す".to_owned(),
                ..NewTask::default()
            })
            .unwrap();
        let dropped = service
            .create_task(NewTask {
                title: "消す".to_owned(),
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
            .move_task(dropped.id, MoveRequest::to_column(BoardColumn::Today))
            .unwrap();
        service.delete_task(dropped.id).unwrap();
        (kept.id, dropped.id)
    };

    // 再オープンしてもボードから消えたまま、削除済み一覧には残る。
    let service = make_service(open(&dir));
    let board = service.board().unwrap();
    assert!(board.columns.today.is_empty());
    assert_eq!(board.columns.new.len(), 1);
    assert_eq!(board.columns.new[0].task.id, kept);

    let deleted = service.list_deleted().unwrap();
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].task.id, dropped);
    // 削除時のステータス・予定は残っているので「元どこにいたか」が分かる。
    assert_eq!(deleted[0].task.status, TaskStatus::Todo);
    assert_eq!(deleted[0].bucket, Some(Bucket::Today));

    // 復元すると元の列へ戻り、リソースも生きている。
    service.restore_task(dropped).unwrap();
    let detail = service.task_detail(dropped).unwrap();
    assert!(!detail.card.task.is_deleted());
    assert_eq!(detail.resources.len(), 1);
    assert_eq!(service.board().unwrap().columns.today.len(), 1);
    assert!(service.list_deleted().unwrap().is_empty());
}

#[test]
fn resources_updates_and_parent_links_persist() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let service = make_service(Arc::clone(&store));

    let parent = service
        .create_task(NewTask {
            title: "親".to_owned(),
            ..NewTask::default()
        })
        .unwrap();
    let child = service
        .create_task(NewTask {
            title: "子".to_owned(),
            is_instant: true,
            resources: vec![NewResource {
                kind: ResourceKind::Url,
                value: "https://example.com/pr/1".to_owned(),
                label: "PR".to_owned(),
                is_primary: true,
            }],
            ..NewTask::default()
        })
        .unwrap();

    service.set_parent(child.id, Some(parent.id)).unwrap();
    service
        .add_task_update(child.id, "レビュー待ち", Origin::Mcp)
        .unwrap();
    service
        .add_resource(
            child.id,
            NewResource {
                kind: ResourceKind::File,
                value: "C:/tmp/log.txt".to_owned(),
                label: String::new(),
                is_primary: false,
            },
        )
        .unwrap();

    let detail = service.task_detail(child.id).unwrap();
    assert_eq!(detail.resources.len(), 2);
    assert_eq!(detail.updates.len(), 1);
    assert_eq!(detail.updates[0].origin, Origin::Mcp);
    assert_eq!(detail.parent.as_ref().map(|p| p.task.id), Some(parent.id));

    let parent_detail = service.task_detail(parent.id).unwrap();
    assert_eq!(parent_detail.children.len(), 1);
    assert_eq!(parent_detail.card.child_count, 1);

    // 再オープン後も保持されている。
    drop(service);
    drop(store);
    let reopened = make_service(open(&dir));
    let detail = reopened.task_detail(child.id).unwrap();
    assert_eq!(detail.resources.len(), 2);
    assert!(detail.resources.iter().filter(|r| r.is_primary).count() == 1);
    assert_eq!(detail.updates.len(), 1);
    assert_eq!(detail.card.task.parent_id, Some(parent.id));
    assert!(detail.card.task.is_instant);
}

#[test]
fn deleting_a_resource_removes_only_that_row() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let service = make_service(Arc::clone(&store));
    let task = service
        .create_task(NewTask {
            title: "資料".to_owned(),
            ..NewTask::default()
        })
        .unwrap();
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
    service
        .add_resource(
            task.id,
            NewResource {
                kind: ResourceKind::Url,
                value: "https://example.com/b".to_owned(),
                label: String::new(),
                is_primary: false,
            },
        )
        .unwrap();

    service.remove_resource(task.id, first.id).unwrap();
    let remaining = store.list_resources(task.id).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].value, "https://example.com/b");
    assert_eq!(store.list_all_resources().unwrap().len(), 1);
}

#[test]
fn settings_are_persisted_and_reloaded() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = open(&dir);
        store
            .set_settings(
                CORE_NAMESPACE,
                &CoreSettings {
                    week_start: WeekStart::Sunday,
                    backup_generations: 7,
                    ..CoreSettings::default()
                },
            )
            .unwrap();
    }
    let store = open(&dir);
    let settings: CoreSettings = store.get_settings(CORE_NAMESPACE).unwrap();
    assert_eq!(settings.week_start, WeekStart::Sunday);
    assert_eq!(settings.backup_generations, 7);
}

#[test]
fn backup_contains_the_data_and_keeps_generations() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let service = make_service(Arc::clone(&store));
    let task = service
        .create_task(NewTask {
            title: "バックアップ対象".to_owned(),
            ..NewTask::default()
        })
        .unwrap();

    let backups_dir = dir.path().join("backups");
    let path = backup::create_backup(&store, &backups_dir, 14).unwrap();
    assert!(path.exists());

    // バックアップを開き直すと同じデータが読める。
    let restored = SqliteStore::open(&path).unwrap();
    let restored_task = restored
        .find_task(task.id)
        .unwrap()
        .expect("複製されている");
    assert_eq!(restored_task.title, "バックアップ対象");
    assert_eq!(restored.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);

    // 世代数を超えたら古いものが消える。
    drop(restored);
    for _ in 0..3 {
        backup::create_backup(&store, &backups_dir, 2).unwrap();
    }
    assert!(backup::list_backups(&backups_dir).unwrap().len() <= 2);
}

#[test]
fn foreign_keys_cascade_resources_on_task_delete() {
    // タスク削除は通常行わないが、ON DELETE CASCADE が効くこと(=foreign_keys=ON)を確認する。
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let service = make_service(Arc::clone(&store));
    let task = service
        .create_task(NewTask {
            title: "削除される".to_owned(),
            resources: vec![NewResource {
                kind: ResourceKind::Url,
                value: "https://example.com".to_owned(),
                label: String::new(),
                is_primary: true,
            }],
            ..NewTask::default()
        })
        .unwrap();
    assert_eq!(store.list_all_resources().unwrap().len(), 1);

    // 別接続から直接削除する(アプリ側に削除 API は無いため)。
    let conn = rusqlite::Connection::open(dir.path().join("data.db")).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    conn.execute("DELETE FROM tasks WHERE id = ?1", [task.id.to_string()])
        .unwrap();
    drop(conn);

    assert_eq!(store.list_all_resources().unwrap().len(), 0);
}
