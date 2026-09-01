//! 関連リソースの URL を、メインウィンドウの中に埋め込んだ**ブラウザペイン**で開く。
//!
//! 既定の「開く」は従来どおり OS の既定ブラウザ(`tauri-plugin-opener`)。こちらは
//! 「ボードを見ながら参照したい」ときの選択肢で、`urlOpenMode` 設定で既定の挙動も選べる。
//!
//! ## 実体は main ウィンドウの子 webview
//!
//! 別ウィンドウではなく、Tauri v2 のマルチ webview(`unstable` feature)を使って
//! main ウィンドウに子 webview(ラベル [`BROWSER_PANE`])を重ねる。
//! 生成は [`tauri::window::Window::add_child`]、以後の矩形更新は
//! [`tauri::webview::Webview::set_bounds`]。
//!
//! 子 webview は**ネイティブの子ウィンドウ**なので、HTML の重なり順(z-index)には
//! 従わず、常に親 webview の内容より前面に描かれる。ペインに重なる UI
//! (モーダル・右クリックメニュー・設定画面)は、開いている間だけ
//! [`browser_pane_set_visible`] でペインを隠して避ける(閉じるのではなく hide なので、
//! 読み込み済みのページとスクロール位置はそのまま残る)。
//! タスク詳細ドロワーだけは隠さない — 「詳細を見ながらページを見る」のが
//! `urlOpenMode: internalAuto` の狙いなので、代わりに**重ならない幅**に収まるよう
//! フロントの CSS で上限を掛けている。
//!
//! 矩形はフロントが持つ。React 側がペイン領域の実寸を ResizeObserver で測り、
//! 論理ピクセルで [`browser_pane_set_bounds`] に送る(DPI は Tauri が
//! [`tauri::LogicalPosition`] / [`tauri::LogicalSize`] から換算する)。
//!
//! ## セキュリティ
//!
//! ここに載るのは**第三者のコンテンツ**なので、Tauri の IPC には触らせない。
//! 守りは 3 枚あり、どれか 1 枚が破れても IPC には届かないようにしてある。
//!
//! 1. **capability は webview ラベルで配る。** 子 webview は main ウィンドウの中にいるので、
//!    capability が `"windows": ["main"]` だと**親のウィンドウラベル経由で main の全権限が
//!    そのまま渡ってしまう**(`RuntimeAuthority::resolve_access` は webview ラベルと
//!    ウィンドウラベルのどちらかが一致すれば通す)。そのため capability は
//!    `"webviews": [...]` で書き、`browser-pane` はどこにも載せない。
//! 2. **リモート生成元は capability の対象外。** 外部 URL の webview からの invoke は
//!    `Origin::Remote` になり、`remote` 節を持たない capability(= 本アプリの全部)とは
//!    照合されない。`remote` 節は足さないこと。
//! 3. **URL を絞る。** 受け付けるのは `http` / `https` だけで、さらに `*.localhost` を弾く。
//!    Windows では questloom 自身が `http://tauri.localhost` から配信されるため、
//!    そこを開くと「ローカル生成元」と見なされてしまう(1 と 2 をまとめて回避されうる)。
//!
//! `dangerousRemoteDomainIpcAccess` は使わない。これらの守りを一括で無効化する設定なので、
//! 設定ごと持たない。

use serde::Deserialize;
use tauri::webview::WebviewBuilder;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Rect, WebviewUrl};
use url::Url;

use crate::commands::{fail, CommandResult};
use crate::window::MAIN_WINDOW;

/// 埋め込みブラウザペイン(子 webview)のラベル。
///
/// **capability の `webviews` には絶対に書かないこと。**
pub const BROWSER_PANE: &str = "browser-pane";

/// 矩形の最小サイズ (論理 px)。0 サイズの webview は作れないので下限を設ける。
const MIN_SIDE: f64 = 1.0;

/// フロントから矩形が来なかったときに使う、ペイン幅の割合。
const FALLBACK_WIDTH_RATIO: f64 = 0.45;

/// フロントから矩形が来なかったときに使う、上端のオフセット (論理 px)。
/// タイトルバー + ペインヘッダのおおよその高さ。
const FALLBACK_TOP: f64 = 80.0;

/// ペインの矩形(main ウィンドウのクライアント領域を原点とする論理ピクセル)。
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaneBounds {
    /// 左端。
    pub x: f64,
    /// 上端。
    pub y: f64,
    /// 幅。
    pub width: f64,
    /// 高さ。
    pub height: f64,
}

impl PaneBounds {
    /// 実際に webview へ渡せる値に整える。
    ///
    /// レイアウト途中の NaN や、閉じかけの 0 幅がそのまま来ても落ちないよう、
    /// 非有限な座標は 0、サイズは [`MIN_SIDE`] 以上に丸める。
    #[must_use]
    fn sanitized(self) -> Self {
        let coord = |value: f64| if value.is_finite() { value } else { 0.0 };
        let side = |value: f64| {
            if value.is_finite() {
                value.max(MIN_SIDE)
            } else {
                MIN_SIDE
            }
        };
        Self {
            x: coord(self.x),
            y: coord(self.y),
            width: side(self.width),
            height: side(self.height),
        }
    }

    fn position(self) -> LogicalPosition<f64> {
        LogicalPosition::new(self.x, self.y)
    }

    fn size(self) -> LogicalSize<f64> {
        LogicalSize::new(self.width, self.height)
    }

    fn rect(self) -> Rect {
        Rect {
            position: self.position().into(),
            size: self.size().into(),
        }
    }
}

/// ブラウザペインで開けなかった理由。
#[derive(Debug, thiserror::Error)]
pub enum BrowserError {
    /// URL として解釈できない。
    #[error("URL として読めません: {0}")]
    Malformed(String),
    /// http / https 以外のスキーム。
    #[error("内蔵ブラウザで開けるのは http / https だけです: {0}")]
    UnsupportedScheme(String),
    /// questloom 自身が使うローカル生成元。
    #[error("内蔵ブラウザで questloom 自身のページ ({0}) は開けません")]
    AppOrigin(String),
    /// メインウィンドウがまだ無い(通常は起こらない)。
    #[error("メインウィンドウが見つかりません")]
    NoMainWindow,
}

/// ブラウザペインで開いてよい URL かを検査し、正規化したものを返す。
///
/// - スキームは `http` / `https` のみ。`file:` / `javascript:` / `data:` や Tauri の
///   内部スキームは弾く。タスクのリソース欄は MCP・AI・プラグインからも書けるので、
///   ここが実質の入口検査になる。
/// - ホストが `*.localhost` のものも弾く。Windows では questloom 自身が
///   `http://tauri.localhost` から配信されるので、そこを開くと Tauri から
///   「ローカル生成元」に見え、main ウィンドウ向けの権限に手が届いてしまう。
///
/// # Errors
/// URL として読めない場合、スキームが http / https でない場合、
/// ホストが `*.localhost` の場合。
pub fn parse_external_url(url: &str) -> Result<Url, BrowserError> {
    let parsed = Url::parse(url.trim()).map_err(|_| BrowserError::Malformed(url.to_owned()))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(BrowserError::UnsupportedScheme(parsed.scheme().to_owned()));
    }
    if let Some(host) = parsed.host_str() {
        let host = host.to_ascii_lowercase();
        if host.ends_with(".localhost") {
            return Err(BrowserError::AppOrigin(host));
        }
    }
    Ok(parsed)
}

/// ブラウザペインを開く(既にあれば URL を差し替えて表示する)。
///
/// `bounds` はフロントが測ったペイン領域。省略時は main ウィンドウの左側に
/// おおよその矩形を取る(初回描画までのつなぎ)。
///
/// **`async` にしてあるのは必須。** `Window::add_child` は Windows でメインスレッドから
/// 呼ぶとデッドロックする(WebView2 の既知の問題)。Tauri は `async` command を
/// 別スレッドで走らせるので、そこからなら安全に呼べる。
///
/// # Errors
/// URL が許可されていない場合、メインウィンドウが無い場合、webview の生成に失敗した場合。
#[tauri::command]
pub async fn browser_pane_open(
    app: AppHandle,
    url: String,
    bounds: Option<PaneBounds>,
) -> CommandResult<()> {
    let target = parse_external_url(&url).map_err(fail)?;

    if let Some(webview) = app.get_webview(BROWSER_PANE) {
        tracing::debug!(%target, "ブラウザペインの URL を差し替えます");
        webview.navigate(target).map_err(fail)?;
        if let Some(bounds) = bounds {
            webview
                .set_bounds(bounds.sanitized().rect())
                .map_err(fail)?;
        }
        webview.show().map_err(fail)?;
        return Ok(());
    }

    let window = app
        .get_window(MAIN_WINDOW)
        .ok_or(BrowserError::NoMainWindow)
        .map_err(fail)?;
    let bounds = bounds.unwrap_or_else(|| fallback_bounds(&app)).sanitized();
    tracing::debug!(%target, ?bounds, "ブラウザペインを生成します");
    window
        .add_child(
            WebviewBuilder::new(BROWSER_PANE, WebviewUrl::External(target)),
            bounds.position(),
            bounds.size(),
        )
        .map_err(fail)?;
    Ok(())
}

/// ブラウザペインを閉じる(子 webview を破棄する)。開いていなければ何もしない(冪等)。
///
/// # Errors
/// webview の破棄に失敗した場合。
#[tauri::command]
pub async fn browser_pane_close(app: AppHandle) -> CommandResult<()> {
    if let Some(webview) = app.get_webview(BROWSER_PANE) {
        tracing::debug!("ブラウザペインを閉じます");
        webview.close().map_err(fail)?;
    }
    Ok(())
}

/// ブラウザペインの矩形を更新する。開いていなければ何もしない。
///
/// フロントのレイアウト(ResizeObserver・ウィンドウのリサイズ)に追従させるための入口。
///
/// # Errors
/// 矩形の適用に失敗した場合。
#[tauri::command]
pub async fn browser_pane_set_bounds(app: AppHandle, bounds: PaneBounds) -> CommandResult<()> {
    if let Some(webview) = app.get_webview(BROWSER_PANE) {
        webview
            .set_bounds(bounds.sanitized().rect())
            .map_err(fail)?;
    }
    Ok(())
}

/// ブラウザペインの表示・非表示を切り替える。開いていなければ何もしない。
///
/// 子 webview はネイティブの子ウィンドウなので HTML の上に必ず描かれる。
/// ドロワーやモーダルを開く間だけ隠すために使う(閉じないので、戻したときに
/// ページの読み込み直しは起きない)。
///
/// # Errors
/// 表示状態の変更に失敗した場合。
#[tauri::command]
pub async fn browser_pane_set_visible(app: AppHandle, visible: bool) -> CommandResult<()> {
    if let Some(webview) = app.get_webview(BROWSER_PANE) {
        if visible {
            webview.show().map_err(fail)?;
        } else {
            webview.hide().map_err(fail)?;
        }
    }
    Ok(())
}

/// フロントから矩形が来なかったときの当て推量(main ウィンドウの左側)。
fn fallback_bounds(app: &AppHandle) -> PaneBounds {
    let size = app
        .get_webview_window(MAIN_WINDOW)
        .and_then(|window| {
            let scale = window.scale_factor().ok()?;
            let inner = window.inner_size().ok()?;
            Some(inner.to_logical::<f64>(scale))
        })
        .unwrap_or(LogicalSize::new(1280.0, 800.0));
    PaneBounds {
        x: 0.0,
        y: FALLBACK_TOP,
        width: size.width * FALLBACK_WIDTH_RATIO,
        height: (size.height - FALLBACK_TOP).max(MIN_SIDE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_and_https_are_accepted() {
        for url in [
            "https://example.com",
            "http://example.com/path?q=1#frag",
            "https://github.com/reanisz/questloom/pull/1",
            // 素の localhost は questloom の生成元ではないので開ける。
            "http://localhost:3000/",
            "http://127.0.0.1:39150/mcp",
        ] {
            assert!(parse_external_url(url).is_ok(), "{url} は開けるべき");
        }
    }

    #[test]
    fn other_schemes_are_rejected() {
        for url in [
            "file:///C:/secret.txt",
            "javascript:alert(1)",
            "data:text/html,<h1>hi</h1>",
            "tauri://localhost",
            "ipc://localhost",
            "mailto:someone@example.com",
        ] {
            assert!(
                matches!(
                    parse_external_url(url),
                    Err(BrowserError::UnsupportedScheme(_))
                ),
                "{url} は拒否するべき"
            );
        }
    }

    /// questloom 自身の生成元は開かせない。
    ///
    /// Windows のアセット同梱ビルドはここから配信されるので、開けてしまうと
    /// 「リモート生成元だから capability の対象外」という守りをすり抜けられる。
    #[test]
    fn the_app_origin_is_rejected() {
        for url in [
            "http://tauri.localhost/",
            "https://tauri.localhost/index.html",
            "http://ipc.localhost/",
            "http://asset.localhost/x",
            "http://TAURI.LOCALHOST/",
        ] {
            assert!(
                matches!(parse_external_url(url), Err(BrowserError::AppOrigin(_))),
                "{url} は拒否するべき"
            );
        }
    }

    #[test]
    fn relative_and_empty_input_is_rejected() {
        for url in ["", "   ", "example.com", "/path/only"] {
            assert!(
                matches!(parse_external_url(url), Err(BrowserError::Malformed(_))),
                "{url:?} は URL として読めないべき"
            );
        }
    }

    #[test]
    fn surrounding_whitespace_is_ignored() {
        let parsed = parse_external_url("  https://example.com/a  ").expect("開けるべき");
        assert_eq!(parsed.as_str(), "https://example.com/a");
    }

    /// エラーはそのままフロントへ出るので、理由が分かる文言にしておく。
    #[test]
    fn errors_explain_the_reason() {
        let error = parse_external_url("file:///C:/secret.txt").unwrap_err();
        assert!(error.to_string().contains("http"));
        let error = parse_external_url("http://tauri.localhost/").unwrap_err();
        assert!(error.to_string().contains("questloom"));
    }

    /// レイアウト途中の壊れた値でも webview に渡せる形へ丸める。
    #[test]
    fn bounds_are_sanitized() {
        let bounds = PaneBounds {
            x: f64::NAN,
            y: 12.0,
            width: 0.0,
            height: f64::INFINITY,
        }
        .sanitized();
        assert_eq!(bounds.x, 0.0);
        assert_eq!(bounds.y, 12.0);
        assert_eq!(bounds.width, MIN_SIDE);
        assert_eq!(bounds.height, MIN_SIDE);
    }

    #[test]
    fn bounds_deserialize_from_camel_case() {
        let bounds: PaneBounds =
            serde_json::from_str(r#"{"x":10,"y":20,"width":300,"height":400}"#).unwrap();
        assert_eq!(
            bounds,
            PaneBounds {
                x: 10.0,
                y: 20.0,
                width: 300.0,
                height: 400.0
            }
        );
    }
}
