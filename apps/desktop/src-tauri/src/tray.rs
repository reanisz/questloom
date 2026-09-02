//! タスクトレイ常駐。
//!
//! 左クリックでメインウィンドウをトグルし、メニューは「開く」「終了」の 2 つ。
//! アプリを本当に終了できるのはトレイメニューの「終了」だけ。

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Runtime};

use crate::window;

/// トレイアイコンの ID。
const TRAY_ID: &str = "questloom";
/// 通常時のツールチップ。
const TOOLTIP: &str = "questloom";
/// 「開く」メニュー項目の ID。
const MENU_OPEN: &str = "open";
/// 「終了」メニュー項目の ID。
const MENU_QUIT: &str = "quit";

/// トレイアイコンを構築して常駐させる。
///
/// # Errors
/// メニュー・トレイアイコンの生成に失敗した場合。
pub fn setup<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, MENU_OPEN, "開く", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "終了", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip(TOOLTIP)
        .menu(&menu)
        // 左クリックはウィンドウのトグルに使うため、メニューは右クリックのみで出す。
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            MENU_OPEN => window::show_main(app, None),
            MENU_QUIT => {
                tracing::info!("トレイメニューから終了します");
                app.exit(0);
            }
            other => tracing::warn!(id = other, "未知のトレイメニュー項目です"),
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                window::toggle_main(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    } else {
        tracing::warn!("既定のウィンドウアイコンが無いため、トレイアイコンは既定表示になります");
    }

    builder.build(app)?;
    tracing::info!("タスクトレイに常駐しました");
    Ok(())
}

/// グローバルショートカットの登録結果をツールチップに反映する。
///
/// 登録に失敗しても(他アプリが同じキーを先に取っている等)アプリは動き続けるので、
/// これまでは warn ログにしか出ておらず、設定画面を開くまで気づけなかった。
/// トレイに載せておけば、押しても反応しないときに理由へ辿り着ける。
pub fn note_shortcut<R: Runtime>(app: &AppHandle<R>, failed_spec: Option<&str>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let tooltip = match failed_spec {
        Some(spec) => format!(
            "{TOOLTIP}(ショートカット {spec} を登録できませんでした。他のアプリが使用中の可能性があります)"
        ),
        None => TOOLTIP.to_owned(),
    };
    if let Err(error) = tray.set_tooltip(Some(&tooltip)) {
        tracing::warn!(%error, "トレイのツールチップを更新できませんでした");
    }
}
