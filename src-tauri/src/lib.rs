mod broadcast;
mod commands;

use commands::*;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_default_config,
            start_teacher_broadcast,
            stop_teacher_broadcast,
            get_teacher_stats,
            start_student_receiver,
            stop_student_receiver,
            is_teacher_broadcasting,
            is_student_receiving,
            get_logs,
            clear_logs,
            test_network_info,
            test_send_packet,
            test_receive_packet,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
