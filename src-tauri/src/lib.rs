mod broadcast;
mod commands;

use commands::*;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            // Config
            get_default_config,
            get_logs,
            clear_logs,
            // Discovery
            start_discovery,
            stop_discovery,
            discovery_announce,
            discovery_query,
            get_discovered_peers,
            get_teachers,
            // Teacher
            start_teacher,
            stop_teacher,
            is_teacher_running,
            // Student (JS rendering - slower)
            start_student,
            stop_student,
            is_student_running,
            // Native Viewer (ultra low latency)
            start_native_viewer,
            stop_native_viewer,
            is_native_viewer_running,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
