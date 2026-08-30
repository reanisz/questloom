//! SQLite Online Backup API による世代付きバックアップ。
//!
//! `backups/questloom-YYYYMMDD-HHMMSS.db` へコピーし、指定世代数を超えた
//! 古いファイルを削除する。ファイル名がローカル時刻の昇順=辞書順になるため、
//! 名前でソートするだけで世代管理ができる。

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Local};
use rusqlite::backup::Backup;
use rusqlite::Connection;

use crate::error::{StoreError, StoreResult};
use crate::SqliteStore;

/// バックアップファイル名の接頭辞。
const PREFIX: &str = "questloom-";
/// バックアップファイル名の拡張子。
const SUFFIX: &str = ".db";
/// 1 回のステップでコピーするページ数。
const PAGES_PER_STEP: std::ffi::c_int = 128;

fn io_error(path: &Path, source: std::io::Error) -> StoreError {
    StoreError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// バックアップを作成し、古い世代を削除する。
///
/// `generations` は残す世代数(今回作成したものを含む)。0 を指定した場合も
/// 最新の 1 世代は残す。
///
/// # Errors
/// ディレクトリ作成・削除の失敗、または SQLite のエラー。
pub fn create_backup(
    store: &SqliteStore,
    backups_dir: &Path,
    generations: u32,
) -> StoreResult<PathBuf> {
    create_backup_at(store, backups_dir, generations, Local::now())
}

/// タイムスタンプを指定してバックアップを作成する(テスト用)。
///
/// # Errors
/// [`create_backup`] と同じ。
pub fn create_backup_at(
    store: &SqliteStore,
    backups_dir: &Path,
    generations: u32,
    now: DateTime<Local>,
) -> StoreResult<PathBuf> {
    std::fs::create_dir_all(backups_dir).map_err(|source| io_error(backups_dir, source))?;

    let destination = unique_destination(backups_dir, now);
    {
        let source = store.conn();
        let mut target = Connection::open(&destination)?;
        let backup = Backup::new(&source, &mut target)?;
        backup.run_to_completion(PAGES_PER_STEP, Duration::from_millis(50), None)?;
    }
    tracing::info!(path = %destination.display(), "バックアップを作成しました");

    prune(backups_dir, generations)?;
    Ok(destination)
}

/// 既存ファイルと衝突しないバックアップ先パスを決める。
fn unique_destination(dir: &Path, now: DateTime<Local>) -> PathBuf {
    let stamp = now.format("%Y%m%d-%H%M%S").to_string();
    let base = dir.join(format!("{PREFIX}{stamp}{SUFFIX}"));
    if !base.exists() {
        return base;
    }
    // 同一秒に複数回実行された場合の保険。
    for n in 1..1000 {
        let candidate = dir.join(format!("{PREFIX}{stamp}-{n:03}{SUFFIX}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    base
}

/// バックアップディレクトリ内のバックアップファイルを名前の昇順で返す。
///
/// # Errors
/// ディレクトリの読み取りに失敗した場合。
pub fn list_backups(dir: &Path) -> StoreResult<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|source| io_error(dir, source))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_backup_file(path))
        .collect();
    files.sort();
    Ok(files)
}

fn is_backup_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(PREFIX) && name.ends_with(SUFFIX))
}

/// 古い世代を削除し、残ったファイルを返す。
///
/// # Errors
/// ディレクトリの読み取りまたはファイル削除に失敗した場合。
pub fn prune(dir: &Path, generations: u32) -> StoreResult<Vec<PathBuf>> {
    let keep = generations.max(1) as usize;
    let files = list_backups(dir)?;
    if files.len() <= keep {
        return Ok(files);
    }
    let remove_count = files.len() - keep;
    for path in files.iter().take(remove_count) {
        std::fs::remove_file(path).map_err(|source| io_error(path, source))?;
        tracing::info!(path = %path.display(), "古いバックアップを削除しました");
    }
    Ok(files.into_iter().skip(remove_count).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32, minute: u32, second: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 8, 31, hour, minute, second)
            .single()
            .expect("一意な時刻")
    }

    #[test]
    fn backup_file_name_follows_the_documented_pattern() {
        let dir = tempfile::tempdir().expect("一時ディレクトリ");
        let store = SqliteStore::open(dir.path().join("data.db")).unwrap();
        let path = create_backup_at(&store, &dir.path().join("backups"), 14, at(9, 5, 3)).unwrap();
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            "questloom-20260831-090503.db"
        );
        assert!(path.exists());
    }

    #[test]
    fn same_second_backups_do_not_overwrite_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("data.db")).unwrap();
        let backups = dir.path().join("backups");
        let first = create_backup_at(&store, &backups, 14, at(9, 5, 3)).unwrap();
        let second = create_backup_at(&store, &backups, 14, at(9, 5, 3)).unwrap();
        assert_ne!(first, second);
        assert_eq!(list_backups(&backups).unwrap().len(), 2);
    }

    #[test]
    fn old_generations_are_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("data.db")).unwrap();
        let backups = dir.path().join("backups");
        for second in 0..5 {
            create_backup_at(&store, &backups, 3, at(9, 0, second)).unwrap();
        }
        let files = list_backups(&backups).unwrap();
        assert_eq!(files.len(), 3, "世代数を超えたら古いものが消える");
        // 残るのは新しい 3 つ。
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec![
                "questloom-20260831-090002.db",
                "questloom-20260831-090003.db",
                "questloom-20260831-090004.db",
            ]
        );
    }

    #[test]
    fn zero_generations_still_keeps_the_latest() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("data.db")).unwrap();
        let backups = dir.path().join("backups");
        create_backup_at(&store, &backups, 0, at(9, 0, 0)).unwrap();
        create_backup_at(&store, &backups, 0, at(9, 0, 1)).unwrap();
        assert_eq!(list_backups(&backups).unwrap().len(), 1);
    }

    #[test]
    fn unrelated_files_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("data.db")).unwrap();
        let backups = dir.path().join("backups");
        std::fs::create_dir_all(&backups).unwrap();
        let keep_me = backups.join("notes.txt");
        std::fs::write(&keep_me, "手で置いたファイル").unwrap();
        for second in 0..4 {
            create_backup_at(&store, &backups, 1, at(9, 0, second)).unwrap();
        }
        assert!(keep_me.exists());
        assert_eq!(list_backups(&backups).unwrap().len(), 1);
    }

    #[test]
    fn list_backups_on_missing_directory_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list_backups(&dir.path().join("nope")).unwrap().is_empty());
    }
}
