//! questloom デスクトップアプリの Tauri シェル。配線のみを担い、ロジックは questloom-core に置く。
//!
//! 起動時に `%APPDATA%\questloom` の DB を開き(マイグレーション + バックアップ)、
//! [`TaskService`](questloom_core::service::TaskService) を State として保持する。
//! データディレクトリと MCP ポートは環境変数で上書きできる([`env_override`])。
//! ドメインイベントは [`contract::TASKS_CHANGED`] として webview へ中継される。
//!
//! ウィンドウは 3 つ。メインウィンドウ(ボード)は閉じるとトレイへ格納され、
//! オーバーレイウィンドウは New タスクがある間だけ表示される。
//! plugin-host ウィンドウは常に非表示で、TS プラグイン([`plugin_host`])を実行する。
//!
//! **3 つとも `tauri.conf.json` で `"create": false` にし、生成は setup フックの中で
//! [`create_windows`] が行う。** Tauri は「conf 定義のウィンドウを生成 → setup フック」の順で
//! 動くため、既定のままだと webview の最初の `invoke` が [`AppState`] の `manage` を
//! 追い越し、`state not managed for field 'state'` で初回描画が失敗しうる
//! (アセット同梱ビルドでは毎回起きる)。属性は conf 側に残したままなので、
//! ウィンドウの見た目・capability の対応づけは従来どおり conf と
//! `capabilities/*.json` で決まる。
//!
//! main ウィンドウにはもう 1 枚、内蔵ブラウザペインの子 webview([`browser`])が
//! 重なることがある。こちらはウィンドウではなく **`Window::add_child` による子 webview** で、
//! 開く URL が決まったときに実行時生成する。第三者のページが載るので、渡す command は
//! Esc の中継([`browser::browser_pane_escape`])**1 つだけ**にする
//! (`capabilities/browser-pane.json`)。
//!
//! **webview ごとの command 許可**は `capabilities/*.json` で決まる。
//! 割り当ては**ウィンドウラベルではなく webview ラベル**(`"webviews": [...]`)で書く。
//! `"windows": ["main"]` にすると、main ウィンドウの中の子 webview
//! (= 外部ページ)にも同じ権限が渡ってしまうため([`browser`] のモジュールドキュメント参照)。
//! アプリ独自 command の一覧は [`app_commands::APP_COMMANDS`] が唯一の定義で、
//! `build.rs` がそこから permission を生成する。下の
//! [`tauri::generate_handler!`] とは 1 対 1 で対応させること
//! (対応が崩れていないことは `tests::app_commands_match_the_capabilities` が見る)。
//!
//! 設定で有効なら、内蔵 MCP サーバー([`mcp`])を `127.0.0.1` で起動する。
//! AI CLI の呼び出し([`ai`])は、実行中の MCP サーバーがあればその URL を CLI に渡す。

pub mod ai;
/// アプリ独自 Tauri command の一覧。`build.rs` と共有する
/// (`include!` される都合でモジュールドキュメントを持てないため、説明はここに置く)。
pub mod app_commands;
pub mod autostart;
pub mod browser;
pub mod commands;
pub mod contract;
pub mod env_override;
pub mod events;
pub mod mcp;
pub mod overlay;
pub mod plugin_host;
pub mod secrets;
pub mod settings;
pub mod shortcut;
pub mod state;
pub mod tray;
pub mod window;

use std::sync::Arc;
use std::time::Duration;

use tauri::{Manager, RunEvent, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;
use tracing_subscriber::EnvFilter;

use crate::mcp::McpSupervisor;
use crate::state::AppState;

/// 終了時に MCP サーバーの停止を待つ上限。
///
/// 停止は axum の graceful shutdown なので、接続中の MCP クライアントがいると
/// 待たされうる。終了操作が固まる方が害なので、待つのは一瞬だけにする。
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// ログを初期化する。`RUST_LOG` があればそれに従う。
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // 二重初期化(テストや複数回呼び出し)でも落ちないよう結果は無視する。
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

/// アプリのエントリポイント。
///
/// # Panics
/// データディレクトリの解決・DB の初期化・Tauri の起動に失敗した場合(いずれも起動時のみ)。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let handle = app.handle().clone();
            // テストからは QUESTLOOM_DATA_DIR で本物の %APPDATA% を避けられる(env_override 参照)。
            let data_dir = match env_override::data_dir() {
                Some(dir) => {
                    tracing::info!(
                        path = %dir.display(),
                        "{} でデータディレクトリを上書きします",
                        env_override::DATA_DIR_ENV
                    );
                    dir
                }
                None => app.path().app_data_dir()?,
            };
            let state = AppState::initialize(&data_dir)?;
            let service = Arc::clone(&state.service);
            let settings = state.settings();
            // MCP の Bearer トークンはコア設定ではなく資格情報ストアにある(crate::secrets)。
            let mcp_token = state.mcp_token();
            app.manage(state);
            app.manage(Arc::new(mcp::McpSupervisor::new(Arc::clone(&service))));
            app.manage(Arc::new(questloom_ai::AiRunner::new()));
            // TS プラグインのライフサイクルは plugin-host webview 上の JS が持つ。
            // Rust 側はそのロード結果を受け取るレジストリだけを用意する。
            app.manage(plugin_host::PluginRegistry::new());

            // State が出揃ってからウィンドウを作る。逆順にすると webview の初回 invoke が
            // manage を追い越して "state not managed" になる(モジュールドキュメント参照)。
            create_windows(app)?;

            tray::setup(&handle)?;

            events::spawn_bridge(handle.clone(), &service);
            events::spawn_day_watcher(Arc::clone(&service));
            overlay::spawn_watcher(handle.clone(), &service);
            settings::spawn_watcher(handle.clone(), &service);

            // 起動時点の設定・タスク状況を反映する。
            settings::apply(&handle, &settings, mcp_token);
            overlay::sync(&handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            // メインウィンドウの閉じるはトレイ格納。終了はトレイメニューからのみ。
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == window::MAIN_WINDOW {
                    api.prevent_close();
                    window::hide_main(window.app_handle());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_board,
            commands::get_task,
            commands::create_task,
            commands::update_task,
            commands::move_task,
            commands::complete_task,
            commands::promote_task,
            commands::add_task_update,
            commands::add_resource,
            commands::remove_resource,
            commands::set_primary_resource,
            commands::set_parent,
            commands::add_checklist_item,
            commands::update_checklist_item,
            commands::remove_checklist_item,
            commands::reorder_checklist_item,
            commands::delete_task,
            commands::restore_task,
            commands::list_deleted_tasks,
            commands::list_archived_done,
            commands::get_settings,
            commands::get_default_settings,
            commands::set_settings,
            commands::get_runtime_status,
            commands::show_main_window,
            browser::browser_pane_open,
            browser::browser_pane_close,
            browser::browser_pane_set_bounds,
            browser::browser_pane_set_visible,
            browser::browser_pane_escape,
            commands::get_mcp_token_status,
            commands::set_mcp_token,
            ai::ai_create_tasks,
            ai::ai_split_task,
            ai::ai_free_instruction,
            ai::ai_cancel,
            plugin_host::plugin_directory,
            plugin_host::plugin_list_sources,
            plugin_host::plugin_kv_get,
            plugin_host::plugin_kv_set,
            plugin_host::plugin_kv_keys,
            plugin_host::plugin_get_settings,
            plugin_host::plugin_set_settings,
            plugin_host::plugin_list_task_resources,
            plugin_host::plugin_log,
            plugin_host::plugin_fetch_allowed,
            plugin_host::plugin_publish_loaded,
            plugin_host::plugin_list_loaded,
            plugin_host::plugin_secret_get,
            plugin_host::plugin_secret_set,
            plugin_host::plugin_secret_status,
        ])
        .build(tauri::generate_context!())
        .expect("Tauri アプリの起動に失敗しました")
        .run(|app, event| {
            // 終了時は MCP サーバーを畳んでからプロセスを落とす(ポートを掴んだままにしない)。
            if matches!(event, RunEvent::Exit) {
                stop_mcp(app);
            }
        });
}

/// `tauri.conf.json` のウィンドウ定義(`app.windows`)からウィンドウを生成する。
///
/// 定義はすべて `"create": false` にしてあるので、Tauri は自動生成しない。
/// ここでは定義をそのまま [`tauri::WebviewWindowBuilder::from_config`] に渡すだけで、
/// サイズ・可視性・フォーカス等の属性は conf 側の記述がそのまま効く。
///
/// 唯一の上乗せが **WebView2 プロファイルの差し替え**
/// ([`env_override::webview_data_dir`])。`QUESTLOOM_DATA_DIR` を指定した起動では
/// localStorage・Cookie・キャッシュも一時ディレクトリへ寄せる。未指定なら何も渡さず、
/// Tauri の既定(`%LOCALAPPDATA%\dev.reanisz.questloom`)のままにする。
///
/// 内蔵ブラウザペイン([`browser`])はこの対象ではない。ウィンドウではなく main の
/// 子 webview で、開く URL が実行時にしか決まらないので、
/// [`browser::browser_pane_open`] が呼ばれたときに生成する
/// (**プロファイルの差し替えは向こうにも同じものが要る**)。
///
/// # Errors
/// ウィンドウの生成に失敗した場合。
fn create_windows(app: &tauri::App) -> tauri::Result<()> {
    // build() が app を借りるので、定義は先に取り出しておく。
    let windows = app.config().app.windows.clone();
    let webview_data_dir = env_override::webview_data_dir();
    if let Some(dir) = &webview_data_dir {
        tracing::info!(
            path = %dir.display(),
            "WebView2 のプロファイルを一時ディレクトリへ寄せます"
        );
    }
    for config in &windows {
        tracing::debug!(label = %config.label, "ウィンドウを生成します");
        let mut builder = tauri::WebviewWindowBuilder::from_config(app.handle(), config)?;
        if let Some(dir) = &webview_data_dir {
            builder = builder.data_directory(dir.clone());
        }
        builder.build()?;
    }
    Ok(())
}

/// 内蔵 MCP サーバーを止める。停止に手間取っても終了を待たせすぎない。
fn stop_mcp(app: &tauri::AppHandle) {
    let Some(supervisor) = app.try_state::<Arc<McpSupervisor>>() else {
        return;
    };
    let supervisor = Arc::clone(supervisor.inner());
    let stopped = tauri::async_runtime::block_on(async move {
        tokio::time::timeout(SHUTDOWN_TIMEOUT, supervisor.stop())
            .await
            .is_ok()
    });
    if !stopped {
        tracing::warn!("MCP サーバーの停止を待ちきれませんでした。そのまま終了します");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::app_commands::{allow_permission, APP_COMMANDS};

    /// capability ファイル 1 件が持つ、アプリ独自 command の permission。
    ///
    /// `core:` / `opener:` のようにプラグインの接頭辞が付くものは対象外。
    fn app_permissions(capability: &str) -> BTreeSet<String> {
        let value: serde_json::Value =
            serde_json::from_str(capability).expect("capability は JSON として読める");
        value["permissions"]
            .as_array()
            .expect("permissions は配列")
            .iter()
            .filter_map(|item| item.as_str())
            .filter(|item| !item.contains(':'))
            .map(ToOwned::to_owned)
            .collect()
    }

    fn allowed(commands: &[&str]) -> BTreeSet<String> {
        commands.iter().map(|c| allow_permission(c)).collect()
    }

    const CONFIG: &str = include_str!("../tauri.conf.json");

    /// ウィンドウは Tauri に自動生成させず、`setup` の中で作る。
    ///
    /// 自動生成(`create` 既定 true)に戻すと、webview の最初の `invoke` が
    /// `app.manage(AppState)` を追い越して `state not managed` になる。
    /// アセット同梱ビルドでは初回描画が必ず失敗するので、ここで固定しておく。
    #[test]
    fn windows_are_created_by_the_setup_hook() {
        let value: serde_json::Value =
            serde_json::from_str(CONFIG).expect("tauri.conf.json は JSON として読める");
        let windows = value["app"]["windows"]
            .as_array()
            .expect("app.windows は配列");
        assert_eq!(windows.len(), 3, "main / overlay / plugin-host の 3 つ");
        for window in windows {
            assert_eq!(
                window["create"],
                serde_json::Value::Bool(false),
                "{} は create: false にすること(生成は crate::create_windows)",
                window["label"]
            );
        }
    }

    /// 標準プラグインは **`examples/plugins/` を正として直接同梱する**。
    ///
    /// `src-tauri/resources/` へコピーを置くと二重管理になり、片方だけ古いまま
    /// 配られる。`bundle.resources` のマッピングでリポジトリ内の実体をそのまま
    /// 配れば、examples を直すだけで同梱版も追随する
    /// (`tauri-build` が `bundle.resources` を cargo の出力ディレクトリへも
    /// コピーするので、`npm run tauri dev` でも同じ内容が読まれる)。
    ///
    /// 配り先は `plugins/` 直下でなければならない。
    /// [`crate::plugin_host::plugin_list_sources`] が読むのは
    /// `<resource_dir>/plugins` **直下**だけなので。
    #[test]
    fn builtin_plugins_are_bundled_straight_from_examples() {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let value: serde_json::Value =
            serde_json::from_str(CONFIG).expect("tauri.conf.json は JSON として読める");
        let resources = value["bundle"]["resources"]
            .as_object()
            .expect("bundle.resources はマッピング(配列ではなく src -> dest の対応)");

        let mut targets = BTreeSet::new();
        for (source, target) in resources {
            let path = manifest_dir.join(source);
            assert!(
                path.is_file(),
                "同梱リソース {source} が見つからない ({})",
                path.display()
            );
            assert!(
                source.starts_with("../../../examples/plugins/"),
                "標準プラグインの実体は examples/plugins/ に置くこと: {source}"
            );
            let target = target.as_str().expect("配り先は文字列");
            assert_eq!(
                std::path::Path::new(target).parent(),
                Some(std::path::Path::new(crate::plugin_host::PLUGINS_DIR)),
                "配り先は plugins/ 直下にすること: {target}"
            );
            targets.insert(target.to_owned());
        }

        // 標準プラグインは GitHub 連携だけ。hello.ts はリポジトリ内のサンプルに留める。
        assert_eq!(
            targets,
            ["plugins/github.ts".to_owned()].into_iter().collect(),
        );

        // コピーを置かない(置くとバージョンが滞留する)。
        assert!(
            !manifest_dir.join("resources").exists(),
            "src-tauri/resources/ は作らないこと。examples/plugins/ を直接同梱する"
        );
    }

    /// 内蔵ブラウザペインは conf に定義せず、IPC の抜け道も開けない。
    ///
    /// - `app.windows` に `browser-pane` を足すと、URL の無いウィンドウが起動時に 1 枚増える
    ///   (実体はウィンドウではなく main の子 webview で、生成は
    ///   [`crate::browser::browser_pane_open`] が実行時に行う)。
    /// - `dangerousRemoteDomainIpcAccess` はリモート生成元へ IPC を開ける唯一の設定なので、
    ///   設定ごと持たない。
    #[test]
    fn the_browser_pane_is_not_configured() {
        let value: serde_json::Value =
            serde_json::from_str(CONFIG).expect("tauri.conf.json は JSON として読める");
        let labels: Vec<&str> = value["app"]["windows"]
            .as_array()
            .expect("app.windows は配列")
            .iter()
            .filter_map(|window| window["label"].as_str())
            .collect();
        assert!(
            !labels.contains(&crate::browser::BROWSER_PANE),
            "browser-pane は conf に定義しないこと(実行時に子 webview として作る)"
        );
        assert!(
            value["app"]["security"]["dangerousRemoteDomainIpcAccess"].is_null(),
            "dangerousRemoteDomainIpcAccess は使わないこと"
        );
    }

    /// 内蔵ブラウザペインの capability。外部ページに渡してよい唯一の窓口。
    const BROWSER_PANE_CAPABILITY: &str = "browser-pane";

    /// capability の割り当ては**ウィンドウラベルではなく webview ラベル**で書く。
    ///
    /// Tauri の `RuntimeAuthority::resolve_access` は「webview ラベルが一致 **または**
    /// ウィンドウラベルが一致」で通す。内蔵ブラウザペインは main ウィンドウの中の
    /// 子 webview なので、`"windows": ["main"]` にすると**外部ページに main の全権限が
    /// 渡ってしまう**。`"webviews"` で配れば、ラベルが違う子 webview は構造的に外れる。
    ///
    /// `browser-pane` の webview ラベルと `remote` 節(= 外部生成元からの invoke を
    /// capability と照合させる指定)を持ってよいのは
    /// [`BROWSER_PANE_CAPABILITY`] だけ。中身が Esc 1 つに閉じていることは
    /// [`the_browser_pane_capability_grants_only_the_escape_command`] が見る。
    ///
    /// `capabilities/` を実際に走査するので、capability ファイルを足しても漏れは拾える
    /// (走査そのものが空振りしていないことは、既知の 4 つと突き合わせて確かめる)。
    #[test]
    fn capabilities_are_granted_by_webview_label() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
        let mut identifiers = BTreeSet::new();
        for entry in std::fs::read_dir(&dir).expect("capabilities/ が読める") {
            let path = entry.expect("capabilities/ のエントリが読める").path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("capability が読める");
            let value: serde_json::Value =
                serde_json::from_str(&text).expect("capability は JSON として読める");
            let identifier = value["identifier"]
                .as_str()
                .expect("identifier がある")
                .to_owned();
            let is_pane = identifier == BROWSER_PANE_CAPABILITY;

            assert!(
                value["windows"].is_null(),
                "{identifier} は windows ではなく webviews で割り当てること(子 webview に権限が漏れる)"
            );
            assert_eq!(
                value["remote"].is_null(),
                !is_pane,
                "remote 節(外部ページからの invoke を通す指定)を持てるのは {BROWSER_PANE_CAPABILITY} だけ"
            );

            let webviews: Vec<&str> = value["webviews"]
                .as_array()
                .expect("webviews は配列")
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect();
            assert_eq!(
                webviews.contains(&crate::browser::BROWSER_PANE),
                is_pane,
                "{identifier} に browser-pane を割り当ててはいけない(外部ページに IPC を渡すことになる)"
            );
            identifiers.insert(identifier);
        }
        let known: BTreeSet<String> =
            ["default", "overlay", "plugin-host", BROWSER_PANE_CAPABILITY]
                .into_iter()
                .map(ToOwned::to_owned)
                .collect();
        assert_eq!(
            identifiers, known,
            "capability を増減したらこのテストの既知一覧も更新すること"
        );
    }

    const MAIN: &str = include_str!("../capabilities/default.json");
    const OVERLAY: &str = include_str!("../capabilities/overlay.json");
    const PLUGIN_HOST: &str = include_str!("../capabilities/plugin-host.json");
    const BROWSER_PANE: &str = include_str!("../capabilities/browser-pane.json");

    /// ブラウザペインに配ってよいのは `browser_pane_escape` **1 つだけ**。
    ///
    /// ここに載るものは、利用者がペインで開いた**任意のページ**が呼べるようになる。
    /// そのため「アプリ独自 command が escape だけ」ではなく
    /// **permissions 配列そのものが 1 要素**であることを見る
    /// (`core:default` のようなプラグイン側の権限が紛れ込むのも防ぐ)。
    #[test]
    fn the_browser_pane_capability_grants_only_the_escape_command() {
        let value: serde_json::Value =
            serde_json::from_str(BROWSER_PANE).expect("capability は JSON として読める");
        let permissions: Vec<&str> = value["permissions"]
            .as_array()
            .expect("permissions は配列")
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect();
        assert_eq!(
            permissions,
            vec![allow_permission("browser_pane_escape")],
            "ペインには Esc の中継以外を渡さないこと(外部ページが呼べるようになる)"
        );
    }

    /// ペインの `remote` に載せる URL パターンは http / https だけ。
    ///
    /// パターンは Tauri が起動時に `RemoteUrlPattern` として解釈するので、
    /// ここでも同じ型で解釈して「http / https のページは一致し、
    /// それ以外のスキームは一致しない」ことを確かめる。
    ///
    /// **明示ポートの URL を必ず入れておくこと。** URLPattern はポート成分を書かないと
    /// 「スキームの既定ポートだけ」に絞られるので、`https://*` と書くと
    /// `https://host:8443/` が静かに外れて、そのページでだけ Esc が効かなくなる。
    #[test]
    fn the_browser_pane_remote_urls_cover_http_and_https_only() {
        use std::str::FromStr;
        use tauri::utils::acl::RemoteUrlPattern;

        let value: serde_json::Value =
            serde_json::from_str(BROWSER_PANE).expect("capability は JSON として読める");
        let patterns: Vec<RemoteUrlPattern> = value["remote"]["urls"]
            .as_array()
            .expect("remote.urls は配列")
            .iter()
            .map(|url| {
                RemoteUrlPattern::from_str(url.as_str().expect("URL は文字列"))
                    .expect("URLPattern として読める")
            })
            .collect();

        let matches = |url: &str| {
            let url = url.parse().expect("URL として読める");
            patterns.iter().any(|pattern| pattern.test(&url))
        };
        for url in [
            "https://github.com/reanisz/questloom/pull/1?tab=files#top",
            "http://example.com/",
            "https://sub.domain.example.co.jp/a/b",
            // 明示ポート付き(社内サーバー・ローカルの開発サーバー)も対象。
            "http://127.0.0.1:8123/page.html",
            "https://example.com:8443/",
        ] {
            assert!(matches(url), "{url} は一致するべき");
        }
        for url in ["tauri://localhost/index.html", "ipc://localhost/x"] {
            assert!(!matches(url), "{url} は一致しないべき");
        }
    }

    /// capability が参照する command permission は、必ず `APP_COMMANDS` から生成されたもの。
    ///
    /// 名前を間違えるとビルド時に `Permission ... not found` で落ちるが、
    /// 逆(command を消したのに capability に残る)も含めてここで押さえる。
    #[test]
    fn capabilities_only_reference_generated_permissions() {
        let known = allowed(APP_COMMANDS);
        for (name, capability) in [
            ("default", MAIN),
            ("overlay", OVERLAY),
            ("plugin-host", PLUGIN_HOST),
            (BROWSER_PANE_CAPABILITY, BROWSER_PANE),
        ] {
            for permission in app_permissions(capability) {
                assert!(
                    known.contains(&permission),
                    "{name} が知らない permission {permission} を参照している"
                );
            }
        }
    }

    /// main ウィンドウだけに配らない command。
    ///
    /// シークレットの**読み出し**は plugin-host 専用にする。プラグインコードは値が
    /// 無いと動かないので読み出しを許すが、設定画面には要らない(設定画面が扱うのは
    /// `plugin_secret_set` / `plugin_secret_status` だけで、値は一度書いたら
    /// アプリから読み出せない)。
    ///
    /// `browser_pane_escape` はブラウザペイン専用の入口。main の中で押した Esc は
    /// `document` のリスナが直接拾うので、main から呼ぶ道は要らない。
    const NOT_FOR_MAIN: &[&str] = &["plugin_secret_get", "browser_pane_escape"];

    /// メインウィンドウ(ボード・ドロワー・設定画面)は
    /// [`NOT_FOR_MAIN`] を除く全 command を使える。
    #[test]
    fn app_commands_match_the_capabilities() {
        let expected: BTreeSet<String> = allowed(APP_COMMANDS)
            .difference(&allowed(NOT_FOR_MAIN))
            .cloned()
            .collect();
        assert_eq!(
            app_permissions(MAIN),
            expected,
            "main の capability は APP_COMMANDS から NOT_FOR_MAIN を除いたものと一致させること"
        );
    }

    /// オーバーレイは New タスクの一覧・完了・メインウィンドウ表示だけ。
    #[test]
    fn the_overlay_window_gets_only_what_it_calls() {
        assert_eq!(
            app_permissions(OVERLAY),
            allowed(&["get_board", "complete_task", "show_main_window"])
        );
    }

    /// plugin-host は「プラグイン基盤 + ctx.tasks が呼ぶタスク操作」まで。
    ///
    /// ここで動くのは第三者のプラグインコードなので、設定・AI・稼働状態は渡さない。
    #[test]
    fn the_plugin_host_window_gets_only_the_plugin_surface() {
        assert_eq!(
            app_permissions(PLUGIN_HOST),
            allowed(&[
                "plugin_list_sources",
                "plugin_kv_get",
                "plugin_kv_set",
                "plugin_kv_keys",
                "plugin_get_settings",
                "plugin_secret_get",
                "plugin_list_task_resources",
                "plugin_log",
                "plugin_fetch_allowed",
                "plugin_publish_loaded",
                "get_board",
                "get_task",
                "create_task",
                "update_task",
                "move_task",
                "complete_task",
                "add_task_update",
                "add_resource",
            ])
        );
    }

    /// 管理系の command は plugin-host / overlay / browser-pane から絶対に見えないこと。
    ///
    /// タスクの削除・復元(`delete_task` / `restore_task` / `list_deleted_tasks`)も
    /// **main だけ**にする。オーバーレイは完了させるだけ、プラグインには
    /// 他人のタスクを消す手段を与えない。過去の完了一覧 (`list_archived_done`) も
    /// ボードの画面のためのものなので main だけに配る(プラグインが完了履歴を
    /// まとめて舐める道は作らない。`get_board` から見えるのは今日の完了まで)。
    ///
    /// チェックリストの command (`*_checklist_item`) も main だけ。これらは origin を
    /// `User` に固定しているので、plugin-host に配ると**第三者のプラグインが
    /// 「利用者の操作」を騙れる**(= 監視中タスクを起こさずに中身を書き換えられる)。
    /// プラグイン・AI がチェックリストを触る道は MCP のツール(origin は `Mcp`)。
    #[test]
    fn management_commands_are_never_exposed_to_untrusted_windows() {
        let forbidden = allowed(&[
            "delete_task",
            "restore_task",
            "list_deleted_tasks",
            "list_archived_done",
            "add_checklist_item",
            "update_checklist_item",
            "remove_checklist_item",
            "reorder_checklist_item",
            "get_settings",
            "get_default_settings",
            "set_settings",
            "get_runtime_status",
            "get_mcp_token_status",
            "set_mcp_token",
            "ai_create_tasks",
            "ai_split_task",
            "ai_free_instruction",
            "ai_cancel",
            "plugin_set_settings",
            "plugin_secret_set",
            "plugin_secret_status",
            "plugin_directory",
            "plugin_list_loaded",
            "browser_pane_open",
            "browser_pane_close",
            "browser_pane_set_bounds",
            "browser_pane_set_visible",
        ]);
        for (name, capability) in [
            ("overlay", OVERLAY),
            ("plugin-host", PLUGIN_HOST),
            (BROWSER_PANE_CAPABILITY, BROWSER_PANE),
        ] {
            let granted = app_permissions(capability);
            for permission in &forbidden {
                assert!(
                    !granted.contains(permission),
                    "{name} に {permission} を渡してはいけない"
                );
            }
        }
    }

    /// シークレットの読み出しは plugin-host だけ。
    ///
    /// 設定画面(main)もオーバーレイも値を読めない。一度書いたシークレットを
    /// アプリの画面から取り出す経路を残さないため。
    #[test]
    fn only_the_plugin_host_can_read_secrets() {
        let reader = allow_permission("plugin_secret_get");
        assert!(app_permissions(PLUGIN_HOST).contains(&reader));
        for (name, capability) in [
            ("default", MAIN),
            ("overlay", OVERLAY),
            (BROWSER_PANE_CAPABILITY, BROWSER_PANE),
        ] {
            assert!(
                !app_permissions(capability).contains(&reader),
                "{name} に {reader} を渡してはいけない"
            );
        }
    }
}
