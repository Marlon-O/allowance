#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--claude-statusline") {
        // The same signed native executable doubles as the helper, before any UI starts.
        ai_usage_widget_lib::claude::helper_main();
        return;
    }
    ai_usage_widget_lib::run();
}
