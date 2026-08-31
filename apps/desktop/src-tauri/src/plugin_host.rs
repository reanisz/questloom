//! TypeScript プラグインホストの Rust 側配線。
//!
//! プラグインのライフサイクル(ロード・activate・dispose・ポーリング)はすべて
//! 非表示の `plugin-host` webview 上の JS が持つ。Rust 側が担うのは
//!
//! 1. プラグインファイル(`<app_data_dir>/plugins/*.ts|*.js`)の列挙、
//! 2. プラグイン用の永続化(`plugin_kv` テーブル / `settings` の `plugin:<id>` 名前空間)、
//! 3. タスク関連リソースの一括取得(ボード用 `list_all_resources` への薄い委譲)、
//! 4. プラグインのログを tracing へ集約すること、
//! 5. ホストがロード結果を公開するためのレジストリ、
//!
//! の 5 点だけである。
//!
//! セキュリティ上の注意: [`plugin_list_sources`] が読むのはプラグインディレクトリ
//! **直下**のファイルのみで、区切り文字を含む名前は [`is_plugin_file_name`] が拒否する。
//! fetch 先の判定は [`is_fetch_allowed`] に置き、ホスト JS から
//! [`plugin_fetch_allowed`] 経由で使わせる(判定ロジックを 1 箇所に保つため)。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use questloom_core::model::TaskId;
use questloom_core::repository::TaskRepository;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

use crate::commands::CommandResult;
use crate::state::AppState;

/// プラグインファイルを置くディレクトリ名(`<app_data_dir>` からの相対)。
pub const PLUGINS_DIR: &str = "plugins";

/// プラグイン設定の名前空間の接頭辞。`settings` テーブルのキーは `plugin:<id>`。
const SETTINGS_PREFIX: &str = "plugin:";

/// プラグインのロード結果が更新されたことを知らせるイベント名。
pub const PLUGINS_LOADED: &str = "questloom://plugins-loaded";

/// 全プラグインの再読み込みを要求するイベント名(発行元は設定画面)。
pub const PLUGINS_RELOAD: &str = "questloom://plugins-reload";

/// プラグイン設定が外部(設定画面)から変更されたことを知らせるイベント名。
pub const PLUGIN_SETTINGS_CHANGED: &str = "questloom://plugin-settings-changed";

fn fail(error: impl std::fmt::Display) -> String {
    let message = error.to_string();
    tracing::warn!(%message, "プラグイン command でエラーが発生しました");
    message
}

/// ファイル名がプラグインソースとして受け入れられるか。
///
/// ディレクトリを跨がせないため、パス区切り・ドライブ指定・`.` / `..` を含む名前は
/// すべて拒否する。拡張子は `.ts` / `.js`(大文字小文字は無視)のみ許し、
/// 型定義だけの `.d.ts` と、隠しファイル(`.` 始まり)は対象外とする。
#[must_use]
pub fn is_plugin_file_name(name: &str) -> bool {
    if name.is_empty() || name.starts_with('.') {
        return false;
    }
    if name
        .chars()
        .any(|c| matches!(c, '/' | '\\' | ':' | '\0') || c.is_control())
    {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".d.ts") {
        return false;
    }
    lower.ends_with(".ts") || lower.ends_with(".js")
}

/// URL の取得先が manifest の許可ドメインに含まれるか。
///
/// 判定は「スキームが http/https」かつ「ホスト名が許可ドメインのいずれかと
/// **完全一致**」。サブドメインのワイルドカードは持たせない
/// (`api.github.com` を許しても `evil.api.github.com` は許さない)。
/// 比較は ASCII 小文字化した上で行い、末尾のドット(`example.com.`)も落とす。
#[must_use]
pub fn is_fetch_allowed(url: &str, domains: &[String]) -> bool {
    let Some(host) = url_host(url) else {
        return false;
    };
    domains
        .iter()
        .any(|domain| normalize_host(domain) == Some(host.clone()))
}

/// `http(s)://host[:port]/...` からホスト名を取り出す。それ以外のスキームは `None`。
fn url_host(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .or_else(|| {
            let lower = url.to_ascii_lowercase();
            if lower.starts_with("https://") {
                Some(&url["https://".len()..])
            } else if lower.starts_with("http://") {
                Some(&url["http://".len()..])
            } else {
                None
            }
        })?;
    // authority は最初の `/` `?` `#` まで。
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .filter(|part| !part.is_empty())?;
    // 認証情報付き URL (`user:pass@host`) はホストの誤認を招くため拒否する。
    if authority.contains('@') {
        return None;
    }
    // IPv6 リテラルは扱わない(プラグインの用途では不要)。
    if authority.contains('[') {
        return None;
    }
    let host = authority.split(':').next()?;
    normalize_host(host)
}

/// ホスト名を比較用に正規化する。空・非 ASCII は `None`。
fn normalize_host(host: &str) -> Option<String> {
    let host = host.trim().trim_end_matches('.');
    if host.is_empty() || !host.is_ascii() {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

/// プラグインディレクトリのパスを解決し、無ければ作成する。
fn plugins_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("データディレクトリを解決できません: {error}"))?
        .join(PLUGINS_DIR);
    ensure_dir(&dir)?;
    Ok(dir)
}

fn ensure_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|error| {
        format!(
            "プラグインディレクトリを作成できません ({}): {error}",
            dir.display()
        )
    })
}

/// プラグインソース 1 件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSource {
    /// ファイル名(ディレクトリ直下。プラグインの識別に使う)。
    pub file_name: String,
    /// ソース本文(UTF-8)。
    pub source: String,
    /// 最終更新時刻 (RFC 3339 / UTC)。取得できない場合は `None`。
    pub modified_at: Option<DateTime<Utc>>,
}

/// 設定スキーマの項目 1 件。設定画面のフォームはこれから自動生成される。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSettingField {
    /// 設定オブジェクト内のキー。
    pub key: String,
    /// 設定画面に出すラベル。
    pub label: String,
    /// 入力の種類。`string` / `number` / `boolean` / `secret`。
    #[serde(rename = "type")]
    pub field_type: String,
    /// 既定値。未指定は `null`。
    #[serde(default)]
    pub default: Option<Value>,
    /// 補足説明。
    #[serde(default)]
    pub hint: Option<String>,
}

/// プラグインの manifest。JS 側 (`sdk.ts`) の `PluginManifest` と対応する。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    /// 一意な識別子。設定・KV・`origin` の名前空間になる。
    pub id: String,
    /// 表示名。
    pub name: String,
    /// バージョン文字列(表示のみ)。
    #[serde(default)]
    pub version: String,
    /// 説明(表示のみ)。
    #[serde(default)]
    pub description: String,
    /// `ctx.fetch` を許すホスト名(完全一致)。
    #[serde(default)]
    pub fetch_domains: Vec<String>,
    /// 設定スキーマ。
    #[serde(default)]
    pub settings_schema: Vec<PluginSettingField>,
}

/// ホストがロードを試みたプラグイン 1 件の結果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedPlugin {
    /// 読み込み元のファイル名。
    pub file_name: String,
    /// 読み取れた manifest。ロードに失敗した場合は `None`。
    #[serde(default)]
    pub manifest: Option<PluginManifest>,
    /// `activate` まで成功したか。
    #[serde(default)]
    pub active: bool,
    /// 失敗した場合のメッセージ。
    #[serde(default)]
    pub error: Option<String>,
}

/// ホストが公開したロード結果を保持するレジストリ。
///
/// plugin-host ウィンドウはロードのたびに [`plugin_publish_loaded`] で上書きし、
/// 設定画面は [`plugin_list_loaded`] で読む(開いた時点の最新が見えるようにするため、
/// イベントだけに頼らない)。
#[derive(Debug, Default)]
pub struct PluginRegistry {
    loaded: Mutex<Vec<LoadedPlugin>>,
}

impl PluginRegistry {
    /// 空のレジストリを作る。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 現在のロード結果を返す。
    #[must_use]
    pub fn snapshot(&self) -> Vec<LoadedPlugin> {
        self.loaded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// ロード結果を差し替える。
    pub fn replace(&self, plugins: Vec<LoadedPlugin>) {
        *self
            .loaded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = plugins;
    }
}

/// プラグインディレクトリの絶対パスを返す(無ければ作成する)。設定画面の表示に使う。
#[tauri::command]
pub fn plugin_directory(app: AppHandle) -> CommandResult<String> {
    plugins_dir(&app)
        .map(|dir| dir.display().to_string())
        .map_err(fail)
}

/// プラグインディレクトリ直下の `*.ts` / `*.js` を列挙して読み込む。
///
/// ディレクトリが無ければ作成し、空配列を返す。読めなかったファイルは
/// 警告ログを出して読み飛ばす(1 つの壊れたファイルで全体を止めないため)。
#[tauri::command]
pub fn plugin_list_sources(app: AppHandle) -> CommandResult<Vec<PluginSource>> {
    let dir = plugins_dir(&app).map_err(fail)?;
    let entries = std::fs::read_dir(&dir).map_err(|error| {
        fail(format!(
            "プラグインを列挙できません ({}): {error}",
            dir.display()
        ))
    })?;

    let mut sources = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                tracing::warn!(%error, "プラグインディレクトリの走査に失敗しました");
                continue;
            }
        };
        let Some(file_name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !is_plugin_file_name(&file_name) {
            continue;
        }
        // ディレクトリ・シンボリックリンク先の実体が通常ファイルでないものは対象外。
        match entry.file_type() {
            Ok(file_type) if file_type.is_file() => {}
            _ => continue,
        }

        let path = dir.join(&file_name);
        let source = match std::fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) => {
                tracing::warn!(file = %file_name, %error, "プラグインを読めませんでした");
                continue;
            }
        };
        let modified_at = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .map(DateTime::<Utc>::from);

        sources.push(PluginSource {
            file_name,
            source,
            modified_at,
        });
    }
    // ロード順を安定させる(ファイル名昇順)。
    sources.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    tracing::debug!(count = sources.len(), dir = %dir.display(), "プラグインを列挙しました");
    Ok(sources)
}

/// プラグイン専用 KV の値を読む。未保存なら `None`。
#[tauri::command]
pub fn plugin_kv_get(
    state: State<'_, AppState>,
    plugin_id: String,
    key: String,
) -> CommandResult<Option<Value>> {
    state.store.plugin_kv_get(&plugin_id, &key).map_err(fail)
}

/// プラグイン専用 KV に値を書く(upsert)。`null` を書くとキーを削除する。
#[tauri::command]
pub fn plugin_kv_set(
    state: State<'_, AppState>,
    plugin_id: String,
    key: String,
    value: Option<Value>,
) -> CommandResult<()> {
    match value {
        Some(Value::Null) | None => state.store.plugin_kv_delete(&plugin_id, &key).map(|_| ()),
        Some(value) => state.store.plugin_kv_set(&plugin_id, &key, &value),
    }
    .map_err(fail)
}

/// プラグイン専用 KV のキー一覧を返す。
#[tauri::command]
pub fn plugin_kv_keys(state: State<'_, AppState>, plugin_id: String) -> CommandResult<Vec<String>> {
    state.store.plugin_kv_keys(&plugin_id).map_err(fail)
}

/// プラグイン設定(`settings` テーブルの `plugin:<id>`)を読む。未保存なら `{}`。
#[tauri::command]
pub fn plugin_get_settings(state: State<'_, AppState>, plugin_id: String) -> CommandResult<Value> {
    let namespace = format!("{SETTINGS_PREFIX}{plugin_id}");
    let raw = state.store.get_settings_json(&namespace).map_err(fail)?;
    let Some(raw) = raw else {
        return Ok(Value::Object(serde_json::Map::new()));
    };
    // 壊れた JSON でプラグインを起動不能にしないため、空オブジェクトへ落とす。
    match serde_json::from_str::<Value>(&raw) {
        Ok(value) => Ok(value),
        Err(error) => {
            tracing::warn!(plugin = %plugin_id, %error, "プラグイン設定を解釈できません");
            Ok(Value::Object(serde_json::Map::new()))
        }
    }
}

/// プラグイン設定を保存し、`questloom://plugin-settings-changed` を発行する。
///
/// plugin-host はこのイベントを受けて `ctx.settings.onChange` を呼び出す。
#[tauri::command]
pub fn plugin_set_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    plugin_id: String,
    value: Value,
) -> CommandResult<()> {
    let namespace = format!("{SETTINGS_PREFIX}{plugin_id}");
    state.store.set_settings(&namespace, &value).map_err(fail)?;
    if let Err(error) = app.emit(
        PLUGIN_SETTINGS_CHANGED,
        SettingsChangedPayload {
            plugin_id: plugin_id.clone(),
        },
    ) {
        tracing::warn!(plugin = %plugin_id, %error, "プラグイン設定変更の通知に失敗しました");
    }
    Ok(())
}

/// [`PLUGIN_SETTINGS_CHANGED`] のペイロード。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsChangedPayload {
    /// 設定が変わったプラグインの id。
    pub plugin_id: String,
}

/// 全タスクの関連リソース(ボード用 `list_all_resources` の薄い委譲)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginTaskResource {
    /// 所属するタスク。
    pub task_id: TaskId,
    /// 種別 (`url` / `file`)。
    pub kind: String,
    /// URL またはファイルパス。
    pub value: String,
    /// 表示ラベル。
    pub label: String,
    /// 主リソースか。
    pub is_primary: bool,
}

/// 全タスクの関連リソースを返す。GitHub プラグインの PR URL 検出などに使う。
///
/// ボードが使う `list_all_resources` への薄い委譲。並びは `(task_id, sort_order)` 昇順。
#[tauri::command]
pub fn plugin_list_task_resources(
    state: State<'_, AppState>,
) -> CommandResult<Vec<PluginTaskResource>> {
    let resources = state.store.list_all_resources().map_err(fail)?;
    Ok(resources
        .into_iter()
        .map(|resource| PluginTaskResource {
            task_id: resource.task_id,
            kind: resource.kind.as_str().to_owned(),
            value: resource.value,
            label: resource.label,
            is_primary: resource.is_primary,
        })
        .collect())
}

/// プラグインからのログを tracing へ転送する。
///
/// レベルは `debug` / `info` / `warn` / `error`。未知の値は `info` として扱う。
#[tauri::command]
pub fn plugin_log(plugin_id: String, level: String, message: String) {
    match level.as_str() {
        "debug" => tracing::debug!(plugin = %plugin_id, "{message}"),
        "warn" => tracing::warn!(plugin = %plugin_id, "{message}"),
        "error" => tracing::error!(plugin = %plugin_id, "{message}"),
        _ => tracing::info!(plugin = %plugin_id, "{message}"),
    }
}

/// URL が manifest の許可ドメインに含まれるか判定する(`ctx.fetch` のガード)。
///
/// 判定ロジックを Rust 側の 1 箇所に置き、テストで固定するための command。
#[tauri::command]
pub fn plugin_fetch_allowed(url: String, domains: Vec<String>) -> bool {
    is_fetch_allowed(&url, &domains)
}

/// plugin-host がロード結果を公開する。設定画面向けに保持し、イベントで通知する。
#[tauri::command]
pub fn plugin_publish_loaded(
    app: AppHandle,
    registry: State<'_, PluginRegistry>,
    plugins: Vec<LoadedPlugin>,
) -> CommandResult<()> {
    for plugin in &plugins {
        match (&plugin.manifest, &plugin.error) {
            (_, Some(error)) => {
                tracing::error!(file = %plugin.file_name, %error, "プラグインのロードに失敗しました");
            }
            (Some(manifest), None) => {
                tracing::info!(
                    file = %plugin.file_name,
                    id = %manifest.id,
                    version = %manifest.version,
                    active = plugin.active,
                    "プラグインをロードしました"
                );
            }
            (None, None) => {}
        }
    }
    registry.replace(plugins.clone());
    if let Err(error) = app.emit(PLUGINS_LOADED, plugins) {
        tracing::warn!(%error, "プラグインのロード結果の通知に失敗しました");
    }
    Ok(())
}

/// plugin-host が公開した最新のロード結果を返す。設定画面が開いたときに読む。
#[tauri::command]
pub fn plugin_list_loaded(registry: State<'_, PluginRegistry>) -> CommandResult<Vec<LoadedPlugin>> {
    Ok(registry.snapshot())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_plugin_file_names() {
        assert!(is_plugin_file_name("hello.ts"));
        assert!(is_plugin_file_name("github.js"));
        assert!(is_plugin_file_name("My Plugin.TS"));
    }

    #[test]
    fn rejects_path_traversal_and_separators() {
        // ディレクトリを跨がせる可能性のある名前はすべて拒否する。
        assert!(!is_plugin_file_name("../evil.ts"));
        assert!(!is_plugin_file_name("..\\evil.ts"));
        assert!(!is_plugin_file_name("sub/evil.ts"));
        assert!(!is_plugin_file_name("sub\\evil.ts"));
        assert!(!is_plugin_file_name("C:/evil.ts"));
        assert!(!is_plugin_file_name("C:evil.ts"));
        assert!(!is_plugin_file_name(".."));
        assert!(!is_plugin_file_name("."));
        assert!(!is_plugin_file_name(""));
        assert!(!is_plugin_file_name(".hidden.ts"));
        assert!(!is_plugin_file_name("bad\0.ts"));
        assert!(!is_plugin_file_name("bad\n.ts"));
    }

    #[test]
    fn rejects_non_plugin_extensions() {
        assert!(!is_plugin_file_name("readme.md"));
        assert!(!is_plugin_file_name("hello.tsx"));
        assert!(!is_plugin_file_name("hello"));
        // 型定義だけのファイルはプラグイン本体ではない。
        assert!(!is_plugin_file_name("questloom.d.ts"));
    }

    fn domains(list: &[&str]) -> Vec<String> {
        list.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn allows_exact_host_matches_only() {
        let allowed = domains(&["api.github.com", "Example.COM"]);
        assert!(is_fetch_allowed("https://api.github.com/user", &allowed));
        assert!(is_fetch_allowed("https://API.GitHub.com/user", &allowed));
        assert!(is_fetch_allowed(
            "https://api.github.com:443/user",
            &allowed
        ));
        assert!(is_fetch_allowed("http://example.com", &allowed));
        assert!(is_fetch_allowed("https://example.com./x", &allowed));

        // サブドメイン・別ドメインは許さない。
        assert!(!is_fetch_allowed("https://evil.api.github.com/x", &allowed));
        assert!(!is_fetch_allowed("https://github.com/x", &allowed));
        assert!(!is_fetch_allowed(
            "https://api.github.com.evil.test/x",
            &allowed
        ));
    }

    #[test]
    fn rejects_non_http_and_ambiguous_urls() {
        let allowed = domains(&["api.github.com"]);
        assert!(!is_fetch_allowed("file:///C:/secret.txt", &allowed));
        assert!(!is_fetch_allowed("/relative/path", &allowed));
        assert!(!is_fetch_allowed("api.github.com/x", &allowed));
        // 認証情報付き URL はホストを誤認しやすいので一律拒否する。
        assert!(!is_fetch_allowed(
            "https://api.github.com@evil.test/x",
            &allowed
        ));
        // IPv6 リテラルは扱わない。
        assert!(!is_fetch_allowed("https://[::1]/x", &domains(&["::1"])));
        assert!(!is_fetch_allowed("https:///x", &allowed));
    }

    #[test]
    fn rejects_everything_when_no_domain_is_declared() {
        assert!(!is_fetch_allowed("https://api.github.com/x", &[]));
    }

    #[test]
    fn registry_replaces_snapshot() {
        let registry = PluginRegistry::new();
        assert!(registry.snapshot().is_empty());

        registry.replace(vec![LoadedPlugin {
            file_name: "hello.ts".to_owned(),
            manifest: None,
            active: false,
            error: Some("失敗".to_owned()),
        }]);
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].file_name, "hello.ts");
    }

    #[test]
    fn manifest_json_is_camel_case() {
        let manifest = PluginManifest {
            id: "github".to_owned(),
            name: "GitHub".to_owned(),
            version: "0.1.0".to_owned(),
            description: String::new(),
            fetch_domains: vec!["api.github.com".to_owned()],
            settings_schema: vec![PluginSettingField {
                key: "pat".to_owned(),
                label: "PAT".to_owned(),
                field_type: "secret".to_owned(),
                default: Some(Value::String(String::new())),
                hint: None,
            }],
        };
        let json = serde_json::to_value(&manifest).unwrap();
        assert_eq!(json["fetchDomains"][0], "api.github.com");
        assert_eq!(json["settingsSchema"][0]["type"], "secret");

        // JS 側から返ってきた形を読み直せる。
        let back: PluginManifest = serde_json::from_value(json).unwrap();
        assert_eq!(back, manifest);
    }

    #[test]
    fn manifest_tolerates_missing_optional_fields() {
        let manifest: PluginManifest =
            serde_json::from_str(r#"{"id":"x","name":"X"}"#).expect("最小 manifest を読める");
        assert_eq!(manifest.version, "");
        assert!(manifest.fetch_domains.is_empty());
        assert!(manifest.settings_schema.is_empty());
    }

    #[test]
    fn event_names_are_valid_for_tauri() {
        for name in [PLUGINS_LOADED, PLUGINS_RELOAD, PLUGIN_SETTINGS_CHANGED] {
            assert!(name
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '-' | '/' | ':' | '_')));
        }
    }
}
