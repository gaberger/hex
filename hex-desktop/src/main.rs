// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod tray;

use hex_hub_core::HubConfig;
use tracing_subscriber::EnvFilter;

fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let port = std::env::var("HEX_HUB_PORT")
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(hex_hub_core::DEFAULT_PORT);

    let token = std::env::var("HEX_DASHBOARD_TOKEN").ok();

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_window_state::Builder::new().build())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_hub_status,
            commands::get_hub_version,
            commands::open_project,
        ])
        .setup(move |app| {
            // Set up system tray
            tray::create_tray(app.handle())?;

            // Spawn the embedded Axum server as a background tokio task
            let config = HubConfig {
                port,
                bind: "127.0.0.1".to_string(),
                token: token.clone(),
                is_daemon: false,
            };

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tracing::info!("Starting embedded Axum server on port {}", config.port);

                let (router, _state) = hex_hub_core::build_app(&config).await;

                // Write lock file so hex-agent can discover us
                let lock_token = config
                    .token
                    .clone()
                    .unwrap_or_else(|| hex_hub_core::daemon::generate_token());

                let addr = format!("127.0.0.1:{}", config.port);
                match tokio::net::TcpListener::bind(&addr).await {
                    Ok(listener) => {
                        if let Err(e) = hex_hub_core::daemon::write_lock(config.port, &lock_token)
                        {
                            tracing::warn!("Failed to write lock file: {}", e);
                        }

                        tracing::info!(
                            "hex-desktop v{} — Axum server running on http://{}",
                            hex_hub_core::version(),
                            addr
                        );

                        // Notify via system tray that hub is ready
                        if let Ok(notification) =
                            tauri_plugin_notification::NotificationExt::notification(&handle)
                                .builder()
                                .title("hex-hub")
                                .body(format!("Dashboard ready at http://{}", addr))
                                .show()
                        {
                            let _ = notification;
                        }

                        if let Err(e) = hex_hub_core::axum::serve(listener, router).await {
                            tracing::error!("Axum server error: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to bind port {}: {}", config.port, e);
                    }
                }
            });

            Ok(())
        })
        .on_window_event(|_window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                // Clean up lock file on exit
                hex_hub_core::daemon::remove_lock();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running hex-desktop");
}
