pub mod core;
pub mod db;
pub mod model;
pub mod network;
pub mod storage;
use std::sync::Arc;
use tauri::Manager;
#[tauri::command]
async fn dispatch(
    core: tauri::State<'_, Arc<core::Core>>,
    action: String,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    core.inner()
        .dispatch(&action, args)
        .await
        .map_err(|e| format!("{e:#}"))
}
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_focus();
            }
        }))
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            app.manage(core::Core::open(&dir)?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![dispatch])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let c = window.state::<Arc<core::Core>>();
                if c.work.try_lock().is_err() {
                    api.prevent_close();
                    let _ = window
                        .app_handle()
                        .get_webview_window("main")
                        .unwrap()
                        .eval("window.dispatchEvent(new Event('routetransfer-close-blocked'))");
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("RouteTransfer 초기화 실패");
}
