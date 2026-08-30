// Windows のリリースビルドで余分なコンソールウィンドウが出るのを防ぐ。削除しないこと。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    questloom_desktop_lib::run()
}
