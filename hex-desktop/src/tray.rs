use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, Url,
};

use crate::commands::SharedHubState;

pub fn create_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let open_dashboard =
        MenuItem::with_id(app, "open_dashboard", "Open Dashboard", true, None::<&str>)?;
    let open_chat = MenuItem::with_id(app, "open_chat", "Open Chat", true, None::<&str>)?;
    let sep1 = MenuItem::with_id(app, "sep1", "─────────", false, None::<&str>)?;
    let start_agent =
        MenuItem::with_id(app, "start_agent", "Start Agent...", true, None::<&str>)?;
    let stop_all_agents =
        MenuItem::with_id(app, "stop_all_agents", "Stop All Agents", true, None::<&str>)?;
    let sep2 = MenuItem::with_id(app, "sep2", "─────────", false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit hex-desktop", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &open_dashboard,
            &open_chat,
            &sep1,
            &start_agent,
            &stop_all_agents,
            &sep2,
            &quit,
        ],
    )?;

    TrayIconBuilder::new()
        .tooltip("hex-hub — 0 agents active")
        .menu(&menu)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "open_dashboard" => {
                if let Some(window) = app.get_webview_window("main") {
                    let port = hex_nexus::DEFAULT_PORT;
                    let url: Url = format!("http://127.0.0.1:{}/", port).parse().unwrap();
                    let _ = window.navigate(url);
                    let _ = window.set_focus();
                }
            }
            "open_chat" => {
                if let Some(window) = app.get_webview_window("main") {
                    let port = hex_nexus::DEFAULT_PORT;
                    let url: Url = format!("http://127.0.0.1:{}/chat", port).parse().unwrap();
                    let _ = window.navigate(url);
                    let _ = window.set_focus();
                }
            }
            "start_agent" => {
                // Navigate to the dashboard which has the agent spawn UI
                if let Some(window) = app.get_webview_window("main") {
                    let port = hex_nexus::DEFAULT_PORT;
                    let url: Url = format!("http://127.0.0.1:{}/", port).parse().unwrap();
                    let _ = window.navigate(url);
                    let _ = window.set_focus();
                }
            }
            "stop_all_agents" => {
                let handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    stop_all_agents_via_api(&handle).await;
                });
            }
            "quit" => {
                hex_nexus::daemon::remove_lock();
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;

    Ok(())
}

/// Stop all agents by listing them and then terminating each one.
async fn stop_all_agents_via_api(app: &AppHandle) {
    let hub_state: tauri::State<'_, SharedHubState> = app.state();
    let hub = hub_state.lock().await;
    let base = format!("http://127.0.0.1:{}", hub.port);

    // List agents
    let agents = match hub.http.get(format!("{}/api/agents", base)).send().await {
        Ok(resp) => match resp.json::<serde_json::Value>().await {
            Ok(body) => body
                .get("agents")
                .and_then(|a| a.as_array())
                .cloned()
                .unwrap_or_default(),
            Err(_) => return,
        },
        Err(_) => return,
    };

    // Terminate each agent
    for agent in &agents {
        if let Some(id) = agent.get("id").and_then(|v| v.as_str()) {
            let _ = hub
                .http
                .delete(format!("{}/api/agents/{}", base, id))
                .send()
                .await;
        }
    }

    tracing::info!("Stopped {} agent(s) via tray menu", agents.len());
}
