mod commands;
mod effective;
mod error;
mod fs_utils;
mod harness;
mod model;
pub mod mcp;
pub mod security;
pub mod sessions;
pub mod tokenizer;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::scan,
            commands::read_file_content,
            commands::list_destinations,
            commands::move_item,
            commands::delete_item,
            commands::restore,
            commands::save_file,
            commands::bulk,
            commands::bulk_restore,
            commands::mcp_get_disabled,
            commands::mcp_set_disabled,
            commands::mcp_get_policy,
            commands::mcp_set_policy,
            commands::mcp_check_policy,
            commands::security_scan,
            commands::security_baseline_check,
            commands::security_baseline_accept,
            commands::context_budget
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}