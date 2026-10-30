mod checks;
mod commands;
mod ffmpeg;
mod process;
mod source;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::inspect_file,
            commands::convert_file,
            commands::save_outputs,
            commands::ffmpeg_available,
            commands::initial_paths,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
