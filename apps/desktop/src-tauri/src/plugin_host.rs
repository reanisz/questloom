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
//! 6. `type: "secret"` の設定項目を OS の資格情報ストアへ橋渡しすること、
//!
//! の 6 点だけである。
//!
//! プラグインの置き場は 2 つある。**アプリ同梱(標準)**の
//! `<resource_dir>/plugins`(`tauri.conf.json` の `bundle.resources` が
//! `examples/plugins/*.ts` をそのまま配る)と、**利用者配置**の
//! `<app_data_dir>/plugins`。[`plugin_list_sources`] は両方を返し、同じ id が
//! 両方にあった場合は**ユーザー側を勝たせる**(ロード順で表現する。[`merge_sources`])。
//!
//! セキュリティ上の注意: [`plugin_list_sources`] が読むのはプラグインディレクトリ
//! **直下**のファイルのみで、区切り文字を含む名前は [`is_plugin_file_name`] が拒否する。
//! この規則は同梱側にも同じように効かせる。
//! fetch 先の判定は [`is_fetch_allowed`] に置き、ホスト JS から
//! [`plugin_fetch_allowed`] 経由で使わせる(判定ロジックを 1 箇所に保つため)。
//!
//! シークレットは `settings` テーブルには入らない([`crate::secrets`])。
//! 読み出し ([`plugin_secret_get`]) は plugin-host のみ、書き込みと状態確認
//! ([`plugin_secret_set`] / [`plugin_secret_status`])は設定画面(main)のみに
//! capability で配ってある。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use questloom_core::model::TaskId;
use questloom_core::repository::TaskRepository;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager, State};
use url::{Host, Url};

use crate::commands::{fail, CommandResult};
use crate::secrets::{self, SecretKey};
use crate::state::AppState;

pub use crate::contract::{PLUGINS_LOADED, PLUGINS_RELOAD, PLUGIN_SETTINGS_CHANGED};

/// プラグインファイルを置くディレクトリ名。
///
/// 利用者配置は `<app_data_dir>/plugins`、アプリ同梱(標準)は
/// `<resource_dir>/plugins`。どちらも同じ名前を使う。
pub const PLUGINS_DIR: &str = "plugins";

/// プラグイン設定の名前空間の接頭辞。`settings` テーブルのキーは `plugin:<id>`。
const SETTINGS_PREFIX: &str = "plugin:";

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
///
/// パースは [`url`] crate に任せる(手書きの authority 分解より取りこぼしが少ない)。
/// その上でプラグイン向けに次を足す。
///
/// - スキームは `http` / `https` のみ。
/// - 認証情報付き URL (`user:pass@host`) はホストの誤認を招くため一律拒否。
/// - IP リテラル(IPv4 / IPv6)は扱わない。許可ドメインはホスト名で書かせる。
fn url_host(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return None;
    }
    match parsed.host()? {
        Host::Domain(domain) => normalize_host(domain),
        Host::Ipv4(_) | Host::Ipv6(_) => None,
    }
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
///
/// 基準は [`AppState::data_dir`] で、`QUESTLOOM_DATA_DIR` の上書きに追随する。
/// `app_data_dir()` を直接引くと、一時プロファイルで起動したテストが利用者の
/// **本物のプラグイン**を読み込んでしまう(GitHub プラグインが本物の PAT で
/// ポーリングを始めるなど、実害が出うる)。
fn plugins_dir(data_dir: &Path) -> Result<PathBuf, String> {
    let dir = data_dir.join(PLUGINS_DIR);
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

/// アプリ同梱(標準)プラグインのディレクトリを解決する。作成はしない。
///
/// 実体は `tauri.conf.json` の `bundle.resources` が配る `<resource_dir>/plugins`。
///
/// **`npm run tauri dev` でも読める。** `tauri-build` はビルドスクリプトの時点で
/// `bundle.resources` を cargo の出力ディレクトリ(= exe の隣)へコピーし、
/// Windows の `resource_dir` は exe のあるディレクトリを指すため、
/// dev 実行でも同じパスに同梱プラグインが居る。したがって
/// 「dev のときだけリポジトリの examples を読む」ようなフォールバックは持たない
/// (持つと、リポジトリの外へ置いた debug ビルドで挙動が変わる)。
///
/// 解決できない・存在しない場合は `None`。同梱プラグインが 1 件も無いだけで、
/// 利用者配置のプラグインは通常どおり動く。
fn builtin_plugins_dir(app: &AppHandle) -> Option<PathBuf> {
    match app.path().resolve(PLUGINS_DIR, BaseDirectory::Resource) {
        Ok(dir) => Some(dir),
        Err(error) => {
            tracing::warn!(%error, "同梱プラグインのディレクトリを解決できませんでした");
            None
        }
    }
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
    /// アプリ同梱(標準)プラグインか。偽なら利用者がプラグインフォルダに置いたもの。
    pub builtin: bool,
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
    /// アプリ同梱(標準)プラグインとして読み込まれたか。
    #[serde(default)]
    pub builtin: bool,
    /// 同じ id の同梱プラグインを隠して読み込まれた利用者配置版か。
    ///
    /// 真なら設定画面に「同梱版を上書きしている」旨を出す(消せば同梱版に戻る)。
    #[serde(default)]
    pub shadows_builtin: bool,
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
pub fn plugin_directory(state: State<'_, AppState>) -> CommandResult<String> {
    plugins_dir(&state.data_dir)
        .map(|dir| dir.display().to_string())
        .map_err(fail)
}

/// ディレクトリ直下の `*.ts` / `*.js` を列挙して読み込む。
///
/// ディレクトリが存在しない場合は空配列(同梱プラグインを持たないビルドでも
/// 利用者配置のプラグインは動かしたいため、エラーにはしない)。読めなかった
/// ファイルは警告ログを出して読み飛ばす(1 つの壊れたファイルで全体を止めないため)。
///
/// 並びはファイル名昇順。
fn read_sources(dir: &Path, builtin: bool) -> Result<Vec<PluginSource>, String> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let entries = std::fs::read_dir(dir)
        .map_err(|error| format!("プラグインを列挙できません ({}): {error}", dir.display()))?;

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
            builtin,
        });
    }
    // ロード順を安定させる(ファイル名昇順)。
    sources.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    Ok(sources)
}

/// 利用者配置と同梱のソースを、ホストがロードする順に並べる。
///
/// **ユーザー配置が先。** ホスト (`plugin-host/host.ts`) は先に来た方に
/// プラグイン id を確保させるので、この順序がそのまま
/// 「同じ id なら利用者のカスタマイズ版が勝つ」という規則になる
/// (同梱版を丸ごと差し替えたいときに、ファイルを置くだけで済ませるため)。
///
/// ファイル名の重複はここでは落とさない。名前が同じでも id が違えば別物なので、
/// 実際に隠すかどうかは manifest を読めるホスト側が決める。
#[must_use]
pub fn merge_sources(
    mut user: Vec<PluginSource>,
    mut builtin: Vec<PluginSource>,
) -> Vec<PluginSource> {
    user.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    builtin.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    user.append(&mut builtin);
    user
}

/// アプリ同梱(標準)と利用者配置のプラグインソースを列挙して読み込む。
///
/// 利用者のプラグインディレクトリは無ければ作成する。同梱側は読めなくても
/// 警告を出すだけで、利用者配置のプラグインのロードは続ける。
#[tauri::command]
pub fn plugin_list_sources(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<Vec<PluginSource>> {
    let user_dir = plugins_dir(&state.data_dir).map_err(fail)?;
    let user = read_sources(&user_dir, false).map_err(fail)?;

    let builtin_dir = builtin_plugins_dir(&app);
    let builtin = match &builtin_dir {
        Some(dir) => read_sources(dir, true).unwrap_or_else(|error| {
            tracing::warn!(%error, "同梱プラグインを列挙できませんでした");
            Vec::new()
        }),
        None => Vec::new(),
    };

    tracing::debug!(
        user = user.len(),
        builtin = builtin.len(),
        user_dir = %user_dir.display(),
        builtin_dir = %builtin_dir.as_deref().map_or_else(String::new, |dir| dir.display().to_string()),
        "プラグインを列挙しました"
    );
    Ok(merge_sources(user, builtin))
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
///
/// **シークレット (`type: "secret"`) はここを通さない。** 設定画面は
/// [`plugin_secret_set`] を使い、この command には非シークレットの項目だけを渡す。
#[tauri::command]
pub fn plugin_set_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    plugin_id: String,
    value: Value,
) -> CommandResult<()> {
    let namespace = format!("{SETTINGS_PREFIX}{plugin_id}");
    state.store.set_settings(&namespace, &value).map_err(fail)?;
    notify_settings_changed(&app, &plugin_id);
    Ok(())
}

/// [`PLUGIN_SETTINGS_CHANGED`] のペイロード。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsChangedPayload {
    /// 設定が変わったプラグインの id。
    pub plugin_id: String,
}

/// 設定変更を plugin-host へ通知する。失敗しても呼び出し元の操作は成功扱い。
fn notify_settings_changed(app: &AppHandle, plugin_id: &str) {
    if let Err(error) = app.emit(
        PLUGIN_SETTINGS_CHANGED,
        SettingsChangedPayload {
            plugin_id: plugin_id.to_owned(),
        },
    ) {
        tracing::warn!(plugin = %plugin_id, %error, "プラグイン設定変更の通知に失敗しました");
    }
}

/* ------------------------------------------------------------------ シークレット */

/// `settingsSchema` の `type` のうち、資格情報ストアへ回すもの。
const SECRET_FIELD_TYPE: &str = "secret";

/// プラグインのシークレット項目を読む。未設定なら `None`。
///
/// **plugin-host ウィンドウ専用**(capability で main / overlay には渡さない)。
/// プラグインコードが実際に値を使うため、ここだけは読み出しを許す。
#[tauri::command]
pub fn plugin_secret_get(
    state: State<'_, AppState>,
    plugin_id: String,
    key: String,
) -> CommandResult<Option<String>> {
    let secret_key = SecretKey::plugin(&plugin_id, &key).map_err(fail)?;
    state.secrets.get(&secret_key).map_err(fail)
}

/// プラグインのシークレット項目が設定されているかだけを返す。
///
/// 設定画面用。**値は返さない**(main ウィンドウからは読み出せない)。
#[tauri::command]
pub fn plugin_secret_status(
    state: State<'_, AppState>,
    plugin_id: String,
    key: String,
) -> CommandResult<bool> {
    let secret_key = SecretKey::plugin(&plugin_id, &key).map_err(fail)?;
    Ok(state.secrets.get(&secret_key).map_err(fail)?.is_some())
}

/// プラグインのシークレット項目を設定・解除する。設定後の状態を返す。
///
/// `None`(または空白のみ)で解除。設定画面用で、保存先は OS の資格情報ストア。
/// 書けなかった場合は**平文へ落とさずエラーを返す**。
/// 成功したら `questloom://plugin-settings-changed` を発行し、プラグインの
/// `ctx.settings.onChange` に新しい値を読み直させる。
#[tauri::command]
pub fn plugin_secret_set(
    app: AppHandle,
    state: State<'_, AppState>,
    plugin_id: String,
    key: String,
    value: Option<String>,
) -> CommandResult<bool> {
    let secret_key = SecretKey::plugin(&plugin_id, &key).map_err(fail)?;
    let configured =
        secrets::put(state.secrets.as_ref(), &secret_key, value.as_deref()).map_err(fail)?;
    notify_settings_changed(&app, &plugin_id);
    Ok(configured)
}

/// 旧バージョンが `settings` テーブルに平文で残したシークレットを資格情報ストアへ移す。
///
/// **manifest 駆動で行う。** Rust 側は `settingsSchema` を知らないので、
/// どのキーがシークレットかは plugin-host が公開する manifest からしか分からない。
/// `plugin:github` の `pat` だけをハードコードする手もあるが、それでは同梱の例以外の
/// プラグインが救われないので、[`plugin_publish_loaded`] のたびにここを通す
/// (移送するものが無ければ DB も資格情報ストアも触らないので、実質 1 回限り)。
///
/// 書き込みは Rust 側で完結し、値の出どころも DB なので、plugin-host に
/// 設定の書き込み権限を渡す必要はない。
fn migrate_plugin_secrets(state: &AppState, manifest: &PluginManifest) {
    let secret_keys: Vec<&str> = manifest
        .settings_schema
        .iter()
        .filter(|field| field.field_type == SECRET_FIELD_TYPE)
        .map(|field| field.key.as_str())
        .collect();
    if secret_keys.is_empty() {
        return;
    }

    let namespace = format!("{SETTINGS_PREFIX}{}", manifest.id);
    let raw = match state.store.get_settings_json(&namespace) {
        Ok(Some(raw)) => raw,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(plugin = %manifest.id, %error, "プラグイン設定を読めませんでした");
            return;
        }
    };
    let Ok(Value::Object(mut stored)) = serde_json::from_str::<Value>(&raw) else {
        return;
    };

    let mut changed = false;
    for key in secret_keys {
        let Some(Value::String(plain)) = stored.get(key) else {
            continue;
        };
        let plain = plain.trim().to_owned();
        if plain.is_empty() {
            // 値ではないので資格情報ストアへは入れない。設定からは落とす。
            stored.remove(key);
            changed = true;
            continue;
        }
        let secret_key = match SecretKey::plugin(&manifest.id, key) {
            Ok(secret_key) => secret_key,
            Err(error) => {
                tracing::warn!(plugin = %manifest.id, %error, "シークレットを移送できません");
                continue;
            }
        };
        match state.secrets.get(&secret_key) {
            // 資格情報ストア側が正。平文は落とすだけ。
            Ok(Some(_)) => {
                stored.remove(key);
                changed = true;
            }
            Ok(None) => match state.secrets.set(&secret_key, &plain) {
                Ok(()) => {
                    tracing::info!(
                        plugin = %manifest.id,
                        key,
                        "平文のシークレットを資格情報マネージャーへ移しました"
                    );
                    stored.remove(key);
                    changed = true;
                }
                // 平文は消さずに残し、次のロードでやり直す。
                Err(error) => tracing::error!(
                    plugin = %manifest.id,
                    key,
                    %error,
                    "シークレットを資格情報マネージャーへ移せませんでした。設定に平文のまま残ります"
                ),
            },
            Err(error) => {
                tracing::warn!(plugin = %manifest.id, key, %error, "資格情報ストアを読めませんでした");
            }
        }
    }

    if changed {
        if let Err(error) = state.store.set_settings(&namespace, &Value::Object(stored)) {
            tracing::warn!(plugin = %manifest.id, %error, "平文のシークレットを設定から消せませんでした");
        }
    }
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
///
/// あわせて、公開された manifest を頼りに平文シークレットの移送
/// ([`migrate_plugin_secrets`])を試みる。
#[tauri::command]
pub fn plugin_publish_loaded(
    app: AppHandle,
    state: State<'_, AppState>,
    registry: State<'_, PluginRegistry>,
    plugins: Vec<LoadedPlugin>,
) -> CommandResult<()> {
    for plugin in &plugins {
        if let Some(manifest) = &plugin.manifest {
            migrate_plugin_secrets(&state, manifest);
        }
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
                    builtin = plugin.builtin,
                    shadows_builtin = plugin.shadows_builtin,
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
            builtin: false,
            shadows_builtin: false,
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

    // イベント名そのものの妥当性は crate::contract のループテストで見る。

    /// ホスト JS が読む形(camelCase)を固定する。
    #[test]
    fn plugin_source_json_is_camel_case() {
        let modified = DateTime::parse_from_rfc3339("2026-08-31T01:02:03Z")
            .unwrap()
            .with_timezone(&Utc);
        let json = serde_json::to_value(PluginSource {
            file_name: "hello.ts".to_owned(),
            source: "export default {}".to_owned(),
            modified_at: Some(modified),
            builtin: true,
        })
        .unwrap();
        assert_eq!(json["fileName"], "hello.ts");
        assert_eq!(json["source"], "export default {}");
        assert_eq!(json["modifiedAt"], "2026-08-31T01:02:03Z");
        assert_eq!(json["builtin"], true);
        assert_eq!(json.as_object().map(serde_json::Map::len), Some(4));

        // 更新時刻が取れなかったファイルは null(JS 側は省略扱いにする)。
        let json = serde_json::to_value(PluginSource {
            file_name: "hello.ts".to_owned(),
            source: String::new(),
            modified_at: None,
            builtin: false,
        })
        .unwrap();
        assert!(json["modifiedAt"].is_null());
        assert_eq!(json["builtin"], false);
    }

    /// ロード結果の形も camelCase。JS 側の [`LoadedPlugin`] と往復できること。
    #[test]
    fn loaded_plugin_json_is_camel_case() {
        let plugin = LoadedPlugin {
            file_name: "github.ts".to_owned(),
            manifest: None,
            active: true,
            error: None,
            builtin: false,
            shadows_builtin: true,
        };
        let json = serde_json::to_value(&plugin).unwrap();
        assert_eq!(json["fileName"], "github.ts");
        assert_eq!(json["builtin"], false);
        assert_eq!(json["shadowsBuiltin"], true);
        assert_eq!(
            serde_json::from_value::<LoadedPlugin>(json).unwrap(),
            plugin
        );

        // 旧いホスト(フィールドを送ってこない)からの入力も読める。
        let old: LoadedPlugin =
            serde_json::from_str(r#"{"fileName":"hello.ts","active":true}"#).expect("読める");
        assert!(!old.builtin);
        assert!(!old.shadows_builtin);
    }

    // ---- ソースの列挙とマージ ----

    fn write_plugin(dir: &Path, name: &str, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), body).unwrap();
    }

    fn names(sources: &[PluginSource]) -> Vec<(String, bool)> {
        sources
            .iter()
            .map(|source| (source.file_name.clone(), source.builtin))
            .collect()
    }

    #[test]
    fn read_sources_reads_plain_files_and_tags_them() {
        let dir = tempfile::tempdir().unwrap();
        write_plugin(dir.path(), "b.ts", "// b");
        write_plugin(dir.path(), "a.js", "// a");
        // 拡張子・隠しファイル・型定義は対象外(is_plugin_file_name と同じ規則)。
        write_plugin(dir.path(), "readme.md", "no");
        write_plugin(dir.path(), ".hidden.ts", "no");
        write_plugin(dir.path(), "questloom.d.ts", "no");
        // サブディレクトリは辿らない。
        write_plugin(&dir.path().join("nested"), "deep.ts", "no");

        let sources = read_sources(dir.path(), true).expect("読める");
        assert_eq!(
            names(&sources),
            vec![("a.js".to_owned(), true), ("b.ts".to_owned(), true)],
            "ファイル名昇順で、同梱フラグが付く"
        );
        assert_eq!(sources[0].source, "// a");
    }

    /// 同梱プラグインを持たないビルドでもエラーにしない。
    #[test]
    fn read_sources_treats_a_missing_directory_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert!(read_sources(&missing, true)
            .expect("エラーにしない")
            .is_empty());
    }

    /// ロード順 = 優先順。ユーザー配置が同梱より先に来る。
    #[test]
    fn merge_sources_puts_user_plugins_first() {
        let source = |name: &str, builtin: bool| PluginSource {
            file_name: name.to_owned(),
            source: String::new(),
            modified_at: None,
            builtin,
        };
        let merged = merge_sources(
            vec![source("z-user.ts", false), source("github.ts", false)],
            vec![source("github.ts", true), source("a-builtin.ts", true)],
        );
        assert_eq!(
            names(&merged),
            vec![
                // ユーザー配置(ファイル名昇順)。
                ("github.ts".to_owned(), false),
                ("z-user.ts".to_owned(), false),
                // 同梱(ファイル名昇順)。同名でも落とさない: id が同じかはホストが判断する。
                ("a-builtin.ts".to_owned(), true),
                ("github.ts".to_owned(), true),
            ]
        );
    }

    /// 設定変更通知のペイロードも camelCase。
    #[test]
    fn settings_changed_payload_is_camel_case() {
        let json = serde_json::to_value(SettingsChangedPayload {
            plugin_id: "github".to_owned(),
        })
        .unwrap();
        assert_eq!(json["pluginId"], "github");
    }

    // ---- シークレットの移送 ----

    use crate::secrets::{MemorySecretStore, SecretStore};
    use std::sync::Arc;

    /// `settingsSchema` に secret 項目を 1 つ持つ manifest。
    fn secret_manifest(id: &str, key: &str) -> PluginManifest {
        PluginManifest {
            id: id.to_owned(),
            name: id.to_owned(),
            version: String::new(),
            description: String::new(),
            fetch_domains: Vec::new(),
            settings_schema: vec![
                PluginSettingField {
                    key: key.to_owned(),
                    label: key.to_owned(),
                    field_type: SECRET_FIELD_TYPE.to_owned(),
                    default: None,
                    hint: None,
                },
                PluginSettingField {
                    key: "pollIntervalMinutes".to_owned(),
                    label: "間隔".to_owned(),
                    field_type: "number".to_owned(),
                    default: None,
                    hint: None,
                },
            ],
        }
    }

    /// 一時プロファイル + インメモリの資格情報ストアで [`AppState`] を組む。
    fn state_with(dir: &Path, secrets: &Arc<MemorySecretStore>) -> AppState {
        AppState::initialize_with_secrets(
            dir,
            Arc::clone(secrets) as Arc<dyn crate::secrets::SecretStore>,
        )
        .expect("初期化できる")
    }

    fn plugin_settings(state: &AppState, id: &str) -> Value {
        let raw = state
            .store
            .get_settings_json(&format!("{SETTINGS_PREFIX}{id}"))
            .expect("読める")
            .unwrap_or_else(|| "{}".to_owned());
        serde_json::from_str(&raw).expect("JSON")
    }

    #[test]
    fn plaintext_plugin_secrets_are_migrated_into_the_secret_store() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecretStore::new());
        let state = state_with(dir.path(), &secrets);

        // 旧バージョンが平文で保存した状態。
        state
            .store
            .set_settings(
                "plugin:github",
                &serde_json::json!({ "pat": "  ghp_secret  ", "pollIntervalMinutes": 7 }),
            )
            .unwrap();

        let manifest = secret_manifest("github", "pat");
        migrate_plugin_secrets(&state, &manifest);

        assert_eq!(
            secrets
                .get(&SecretKey::plugin("github", "pat").unwrap())
                .unwrap()
                .as_deref(),
            Some("ghp_secret")
        );
        let stored = plugin_settings(&state, "github");
        assert!(stored.get("pat").is_none(), "平文は消える: {stored}");
        assert_eq!(stored["pollIntervalMinutes"], 7, "他の項目は残る");

        // 2 回目は何も起きない(冪等)。
        migrate_plugin_secrets(&state, &manifest);
        assert_eq!(secrets.keys(), vec!["plugin:github/pat".to_owned()]);
    }

    #[test]
    fn migration_keeps_the_value_already_in_the_secret_store() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecretStore::new());
        let state = state_with(dir.path(), &secrets);
        secrets
            .set(&SecretKey::plugin("github", "pat").unwrap(), "current")
            .unwrap();
        state
            .store
            .set_settings("plugin:github", &serde_json::json!({ "pat": "old" }))
            .unwrap();

        migrate_plugin_secrets(&state, &secret_manifest("github", "pat"));

        assert_eq!(
            secrets
                .get(&SecretKey::plugin("github", "pat").unwrap())
                .unwrap()
                .as_deref(),
            Some("current")
        );
        assert!(plugin_settings(&state, "github").get("pat").is_none());
    }

    #[test]
    fn migration_leaves_the_plaintext_when_the_secret_store_is_broken() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::initialize_with_secrets(
            dir.path(),
            Arc::new(MemorySecretStore::failing()) as Arc<dyn crate::secrets::SecretStore>,
        )
        .unwrap();
        state
            .store
            .set_settings("plugin:github", &serde_json::json!({ "pat": "ghp_secret" }))
            .unwrap();

        migrate_plugin_secrets(&state, &secret_manifest("github", "pat"));

        // 次のロードでやり直せるよう、平文は残す。
        assert_eq!(plugin_settings(&state, "github")["pat"], "ghp_secret");
    }

    #[test]
    fn migration_ignores_plugins_without_secret_fields_and_blank_values() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecretStore::new());
        let state = state_with(dir.path(), &secrets);

        // secret 項目を持たない manifest は設定に触らない。
        state
            .store
            .set_settings(
                "plugin:hello",
                &serde_json::json!({ "pat": "not-a-secret" }),
            )
            .unwrap();
        let plain = PluginManifest {
            settings_schema: Vec::new(),
            ..secret_manifest("hello", "pat")
        };
        migrate_plugin_secrets(&state, &plain);
        assert_eq!(plugin_settings(&state, "hello")["pat"], "not-a-secret");
        assert!(secrets.keys().is_empty());

        // 空文字列は値ではないので、ストアには入れず設定から落とすだけ。
        state
            .store
            .set_settings("plugin:github", &serde_json::json!({ "pat": "   " }))
            .unwrap();
        migrate_plugin_secrets(&state, &secret_manifest("github", "pat"));
        assert!(plugin_settings(&state, "github").get("pat").is_none());
        assert!(secrets.keys().is_empty());
    }

    /// 全タスクの関連リソースも camelCase。
    #[test]
    fn plugin_task_resource_json_is_camel_case() {
        let id = TaskId::new();
        let json = serde_json::to_value(PluginTaskResource {
            task_id: id,
            kind: "url".to_owned(),
            value: "https://example.com".to_owned(),
            label: "例".to_owned(),
            is_primary: true,
        })
        .unwrap();
        assert_eq!(json["taskId"], id.to_string());
        assert_eq!(json["isPrimary"], true);
    }
}
