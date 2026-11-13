mod commands;
mod mcp_config;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            commands::clean_previews(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::expand_paths,
            commands::load_images,
            commands::preview,
            commands::export_images,
            commands::find_preset,
            commands::mcp_status,
            commands::mcp_install,
            commands::initial_paths,
        ])
        .run(tauri::generate_context!())
        .expect("启动应用失败");
}
