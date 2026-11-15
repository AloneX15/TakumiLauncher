// TakumiLauncher Desktop Application
// Tauri + Rust backend, Svelte frontend

#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

mod accounts;
mod commands;
mod content;
mod content_commands;
mod forge;
mod install;
mod launch;
mod modpack;
mod private_instances;
mod private_runtime;
mod session;
mod settings;
mod state;
mod web_connection;

#[cfg(test)]
mod e2e;

#[cfg(test)]
mod bench;

use state::AppState;
use std::sync::Arc;
use tauri::Manager;
use tracing::info;

fn main() {
    telemetry::init_telemetry(tracing::Level::INFO);

    tauri::Builder::default()
        .setup(|app| {
            let root = app
                .path_resolver()
                .app_data_dir()
                .ok_or("no se pudo determinar el directorio de datos")?;
            info!(
                "TakumiLauncher v{} — datos en {}",
                app.package_info().version,
                root.display()
            );
            let state = Arc::new(AppState::new(root)?);
            // Event files a crash left decrypted in a private instance.
            private_runtime::scrub_all(&state);
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_info,
            commands::open_data_folder,
            commands::get_versions,
            commands::get_loader_versions,
            commands::list_instances,
            commands::create_instance,
            commands::update_instance,
            commands::delete_instance,
            commands::open_instance_folder,
            commands::list_accounts,
            commands::remove_account,
            commands::set_active_account,
            commands::start_microsoft_login,
            commands::cancel_microsoft_login,
            commands::get_settings,
            commands::save_settings,
            commands::launch_instance,
            commands::stop_instance,
            commands::repair_instance,
            commands::running_instances,
            commands::sync_private_instance,
            private_instances::list_private_instances,
            private_instances::join_private_instance,
            commands::restart_private_instance,
            session::get_session,
            session::takumi_login,
            session::takumi_logout,
            session::get_news,
            session::check_launcher_update,
            web_connection::check_web_connection,
            content_commands::search_content,
            content_commands::list_content,
            content_commands::install_content,
            content_commands::optimize_instance,
            content_commands::set_content_enabled,
            content_commands::remove_content,
            content_commands::check_content_updates,
            content_commands::update_content,
            content_commands::search_modpacks,
            content_commands::install_modpack,
            content_commands::import_modpack,
            content_commands::duplicate_instance,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
