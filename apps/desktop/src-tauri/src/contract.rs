//! フロントエンドと共有する `questloom://` イベント名。
//!
//! Rust 側の定義はこのモジュールだけにあり、[`events`](crate::events) /
//! [`window`](crate::window) / [`ai`](crate::ai) / [`plugin_host`](crate::plugin_host) は
//! ここから `use` する(以前は 4 ファイルに散っていて、名前の妥当性テストも
//! 同じものが 4 本コピーされていた)。
//!
//! TS 側の対応する定義は 2 箇所。**codegen はしていないので、片方を変えたら
//! もう片方も直すこと**。
//!
//! | 定数 | TS 側 |
//! |---|---|
//! | [`TASKS_CHANGED`] / [`OPEN_TASK`] / [`AI_STATUS`] / [`BROWSER_PANE_ESCAPE`] | `apps/desktop/src/api.ts` |
//! | [`PLUGINS_LOADED`] / [`PLUGINS_RELOAD`] / [`PLUGIN_SETTINGS_CHANGED`] | `apps/desktop/src/plugin-host/api.ts` |
//!
//! アプリ独自 command の一覧は [`app_commands`](crate::app_commands) にある
//! (build.rs と共有する都合で別ファイル)。

/// タスク関連の変更を webview へ通知するイベント名。
///
/// フロントはこれを受け取ったら `get_board` などで再フェッチする。
pub const TASKS_CHANGED: &str = "questloom://tasks-changed";

/// メインウィンドウでタスク詳細を開かせるイベント名。
///
/// オーバーレイから通常タスクをクリックしたときに、[`TASKS_CHANGED`] とは別経路で
/// メインウィンドウのみへ送る。
pub const OPEN_TASK: &str = "questloom://open-task";

/// AI 実行の進捗を webview へ通知するイベント名。
pub const AI_STATUS: &str = "questloom://ai-status";

/// 内蔵ブラウザペインの中で Esc が押されたことを、メインウィンドウへ知らせるイベント名。
///
/// 子 webview のキー入力は main の `document` には届かないので、ペインへ注入した
/// スクリプトが [`browser_pane_escape`](crate::browser::browser_pane_escape) を呼び、
/// その command がこのイベントを main だけへ送る。
pub const BROWSER_PANE_ESCAPE: &str = "questloom://browser-pane-escape";

/// プラグインのロード結果が更新されたことを知らせるイベント名。
pub const PLUGINS_LOADED: &str = "questloom://plugins-loaded";

/// 全プラグインの再読み込みを要求するイベント名(発行元は設定画面)。
pub const PLUGINS_RELOAD: &str = "questloom://plugins-reload";

/// プラグイン設定が外部(設定画面)から変更されたことを知らせるイベント名。
pub const PLUGIN_SETTINGS_CHANGED: &str = "questloom://plugin-settings-changed";

/// 定義済みイベント名の全件。名前の妥当性テストはこれを回す。
pub const EVENT_NAMES: &[&str] = &[
    TASKS_CHANGED,
    OPEN_TASK,
    AI_STATUS,
    BROWSER_PANE_ESCAPE,
    PLUGINS_LOADED,
    PLUGINS_RELOAD,
    PLUGIN_SETTINGS_CHANGED,
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Tauri v2 のイベント名に使える文字だけで構成されていること。
    ///
    /// 以前は 4 つのモジュールに同じテストがコピーされていた。定数を 1 箇所に
    /// 集めたので、ここのループ 1 本で全件を見る。
    #[test]
    fn event_names_are_valid_for_tauri() {
        for name in EVENT_NAMES {
            assert!(
                name.starts_with("questloom://"),
                "{name} は questloom:// 名前空間に属していない"
            );
            assert!(
                name.chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '-' | '/' | ':' | '_')),
                "{name} に Tauri v2 が許容しない文字が含まれている"
            );
        }
    }

    #[test]
    fn event_names_are_unique() {
        let mut sorted = EVENT_NAMES.to_vec();
        sorted.sort_unstable();
        let before = sorted.len();
        sorted.dedup();
        assert_eq!(sorted.len(), before, "イベント名が重複している");
    }
}
