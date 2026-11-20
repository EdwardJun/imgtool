mod commands;
mod design_spec;
mod generate;
mod matting;
mod mcp_config;
mod references;
mod smart_crop;
mod svg_icons;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(references::RefCache::default())
        .setup(|app| {
            commands::clean_previews(app.handle());
            matting::register_downloaded();
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
            generate::get_api_settings,
            generate::save_api_settings,
            generate::generate_image,
            generate::save_files,
            generate::enhance_prompt,
            smart_crop::detect_regions,
            smart_crop::export_regions,
            references::upload_reference,
            svg_icons::generate_svg_icons,
            svg_icons::render_svg_set,
            matting::matting_model_status,
            matting::download_matting_model,
            design_spec::analyze_design,
            design_spec::load_design_spec,
            design_spec::save_design_spec,
            design_spec::reanalyze_region,
            design_spec::normalize_spec,
            design_spec::generate_design_icons,
            design_spec::open_folder,
        ])
        .run(tauri::generate_context!())
        .expect("启动应用失败");
}
