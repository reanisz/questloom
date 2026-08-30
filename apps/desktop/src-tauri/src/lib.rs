//! questloom デスクトップアプリの Tauri シェル。配線のみを担い、ロジックは questloom-core に置く。

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
