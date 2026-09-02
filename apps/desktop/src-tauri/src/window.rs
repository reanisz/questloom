//! メインウィンドウの表示制御。
//!
//! トレイ・グローバルショートカット・オーバーレイのいずれからも同じ入口を通す。
//! 閉じる操作は終了ではなくトレイ格納(hide)で、実際の終了はトレイメニューからのみ。
//!
//! **トグルの判定に [`WebviewWindow::is_focused`] を使わないこと。** メインウィンドウは
//! 中に WebView2 の子 HWND を抱えていて、利用者がアプリの中を一度でもクリックすると
//! キーボードフォーカスはその子へ移る。tao の `is_focused` は「tao が作った最上位
//! ウィンドウ自身がフォーカスを持つか」なので、**questloom が前面にいても false を返す**。
//! これを「前面ではない」と読むと、グローバルショートカットが hide 側へ一度も
//! 入らなくなり、前面にいるときに押しても何も起きない(= 効かないように見える)。
//! 代わりに [`is_foreground`] が `GetForegroundWindow` と HWND を突き合わせる。
//!
//! 前面化 (`show_main`) も、Windows のフォーカス窃取防止で `SetForegroundWindow` が
//! 黙って失敗しうる。tao の `set_focus` が効かなかったときのために
//! [`platform::force_foreground`] を後追いで当てる。

use questloom_core::model::TaskId;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindow};

pub use crate::contract::OPEN_TASK;

/// メインウィンドウのラベル(`tauri.conf.json` と一致させること)。
pub const MAIN_WINDOW: &str = "main";

/// [`OPEN_TASK`] のペイロード。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenTask {
    /// 開くタスク。
    pub task_id: TaskId,
}

/// トグル時に見るメインウィンドウの状態。
///
/// `foreground` は「OS が前面と見なしているか」で、キーボードフォーカスが
/// 中の webview にあるかどうかとは無関係(モジュールドキュメント参照)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Presence {
    /// `IsWindowVisible` 相当。最小化中でも真になりうる。
    pub visible: bool,
    /// `IsIconic` 相当。
    pub minimized: bool,
    /// このウィンドウが前面か。
    pub foreground: bool,
}

/// [`toggle_main`] が取る動作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleAction {
    /// トレイへ格納する。
    Hide,
    /// 表示して前面に出す。
    Show,
}

/// 状態からトグルの動作を決める。
///
/// 「見えていて、最小化されておらず、前面にいる」ときだけ格納する。
/// 裏に回っている・最小化されている・隠れている場合は前面へ出す
/// (見えているのに前面ではない状態で隠すと、押しても何も起きないように見えるため)。
#[must_use]
pub fn toggle_action(presence: Presence) -> ToggleAction {
    if presence.visible && !presence.minimized && presence.foreground {
        ToggleAction::Hide
    } else {
        ToggleAction::Show
    }
}

/// メインウィンドウを取得する。まだ生成されていない場合は `None`。
#[must_use]
pub fn main_window<R: Runtime>(app: &AppHandle<R>) -> Option<WebviewWindow<R>> {
    app.get_webview_window(MAIN_WINDOW)
}

/// ウィンドウが前面(フォアグラウンド)にいるか。
///
/// Windows では `GetForegroundWindow` と HWND を直接突き合わせる。
/// `is_focused` は中の webview にフォーカスが移った時点で false になるので使えない。
fn is_foreground<R: Runtime>(window: &WebviewWindow<R>) -> bool {
    #[cfg(windows)]
    {
        if let Ok(hwnd) = window.hwnd() {
            return platform::is_foreground(hwnd.0 as isize);
        }
    }
    // HWND を取れない環境・プラットフォームでは従来どおりの判定に落とす。
    window.is_focused().unwrap_or(false)
}

/// メインウィンドウの現在の状態を読む。
fn presence<R: Runtime>(window: &WebviewWindow<R>) -> Presence {
    Presence {
        visible: window.is_visible().unwrap_or(false),
        minimized: window.is_minimized().unwrap_or(false),
        foreground: is_foreground(window),
    }
}

/// メインウィンドウを表示してフォーカスし、必要ならタスク詳細を開かせる。
pub fn show_main<R: Runtime>(app: &AppHandle<R>, task_id: Option<TaskId>) {
    let Some(window) = main_window(app) else {
        tracing::warn!("メインウィンドウが見つかりません");
        return;
    };
    if window.is_minimized().unwrap_or(false) {
        let _ = window.unminimize();
    }
    if let Err(error) = window.show() {
        tracing::warn!(%error, "メインウィンドウを表示できませんでした");
    }
    if let Err(error) = window.set_focus() {
        tracing::warn!(%error, "メインウィンドウをフォーカスできませんでした");
    }
    // set_focus は Windows のフォーカス窃取防止に当たると黙って失敗する。
    // 前面に出ていなければ、入力キューを繋いでもう一度だけ試す。
    #[cfg(windows)]
    if !is_foreground(&window) {
        if let Ok(hwnd) = window.hwnd() {
            platform::force_foreground(hwnd.0 as isize);
        }
    }
    if let Some(task_id) = task_id {
        if let Err(error) = app.emit_to(MAIN_WINDOW, OPEN_TASK, OpenTask { task_id }) {
            tracing::warn!(%error, "タスクを開く通知に失敗しました");
        }
    }
}

/// メインウィンドウを隠す(トレイ格納)。
pub fn hide_main<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = main_window(app) else {
        return;
    };
    if let Err(error) = window.hide() {
        tracing::warn!(%error, "メインウィンドウを隠せませんでした");
    }
}

/// メインウィンドウの表示状態をトグルする。
///
/// 表示中でも前面でない(最小化・背面)場合は隠さず前面に出す。
pub fn toggle_main<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = main_window(app) else {
        tracing::warn!("メインウィンドウが見つかりません");
        return;
    };
    let presence = presence(&window);
    let action = toggle_action(presence);
    tracing::debug!(
        visible = presence.visible,
        minimized = presence.minimized,
        foreground = presence.foreground,
        ?action,
        "メインウィンドウをトグルします"
    );
    match action {
        ToggleAction::Hide => hide_main(app),
        ToggleAction::Show => show_main(app, None),
    }
}

/// Windows のウィンドウ前面化まわり。
///
/// tao/Tauri の API では届かない 2 点だけをここで扱う。
///
/// - **前面判定**: `is_focused` は tao の最上位ウィンドウがキーボードフォーカスを
///   持つかを見るので、WebView2 の子 HWND にフォーカスがある間は false になる。
/// - **前面化**: Windows は前面でないプロセスからの `SetForegroundWindow` を
///   拒否する。呼び出し元スレッドの入力キューを現在の前面スレッドへ
///   `AttachThreadInput` で繋ぐと、拒否を回避できる(定石)。
#[cfg(windows)]
mod platform {
    use std::ffi::c_void;
    use std::ptr;

    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
    };

    /// `isize` として持ち回している HWND を windows-sys の型へ戻す。
    fn as_hwnd(hwnd: isize) -> HWND {
        hwnd as *mut c_void
    }

    /// このウィンドウが前面か。
    pub fn is_foreground(hwnd: isize) -> bool {
        // 前面ウィンドウは必ず最上位ウィンドウなので、そのまま突き合わせればよい
        // (子 webview にフォーカスがあっても、前面なのは親のメインウィンドウ)。
        let foreground = unsafe { GetForegroundWindow() };
        !foreground.is_null() && foreground == as_hwnd(hwnd)
    }

    /// 前面化を強制する。すでに前面なら何もしない。
    pub fn force_foreground(hwnd: isize) {
        let hwnd = as_hwnd(hwnd);
        unsafe {
            let foreground = GetForegroundWindow();
            if foreground == hwnd {
                return;
            }
            let target_thread = if foreground.is_null() {
                0
            } else {
                GetWindowThreadProcessId(foreground, ptr::null_mut())
            };
            let this_thread = GetCurrentThreadId();
            // 同じスレッド(= 自分が前面)なら繋ぐ必要はない。繋いだ場合は必ず外す。
            let attached = target_thread != 0
                && target_thread != this_thread
                && AttachThreadInput(this_thread, target_thread, 1) != 0;
            SetForegroundWindow(hwnd);
            BringWindowToTop(hwnd);
            SetFocus(hwnd);
            if attached {
                AttachThreadInput(this_thread, target_thread, 0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // イベント名そのものの妥当性は crate::contract のループテストで見る。

    #[test]
    fn open_task_payload_is_camel_case() {
        let id = TaskId::new();
        let json = serde_json::to_value(OpenTask { task_id: id }).unwrap();
        assert_eq!(json["taskId"], id.to_string());
    }

    fn presence(visible: bool, minimized: bool, foreground: bool) -> Presence {
        Presence {
            visible,
            minimized,
            foreground,
        }
    }

    /// 隠すのは「見えていて・最小化されておらず・前面」のときだけ。
    ///
    /// 表は {visible, minimized, foreground} の全 8 通り。`foreground` を
    /// `is_focused` で代用していた頃は、アプリの中をクリックした後に
    /// 常に false になり、前面にいても Hide へ入れなかった
    /// (= ショートカットが効かない)。
    #[test]
    fn only_a_foreground_window_is_hidden() {
        use ToggleAction::{Hide, Show};
        let table = [
            (presence(true, false, true), Hide),
            (presence(true, false, false), Show),
            (presence(true, true, true), Show),
            (presence(true, true, false), Show),
            (presence(false, false, true), Show),
            (presence(false, false, false), Show),
            (presence(false, true, true), Show),
            (presence(false, true, false), Show),
        ];
        for (state, expected) in table {
            assert_eq!(toggle_action(state), expected, "{state:?}");
        }
    }

    /// 最小化されたウィンドウは `IsWindowVisible` が真でも「見えていない」扱い。
    ///
    /// タスクバーから最小化した直後に押したら、隠すのではなく復元してほしい。
    #[test]
    fn a_minimized_window_is_restored_not_hidden() {
        assert_eq!(
            toggle_action(presence(true, true, true)),
            ToggleAction::Show
        );
    }
}
