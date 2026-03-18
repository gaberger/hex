use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, Url,
};

pub fn create_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let open_dashboard = MenuItem::with_id(app, "open_dashboard", "Open Dashboard", true, None::<&str>)?;
    let open_chat = MenuItem::with_id(app, "open_chat", "Open Chat", true, None::<&str>)?;
    let separator = MenuItem::with_id(app, "sep", "─────────", false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit hex-desktop", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&open_dashboard, &open_chat, &separator, &quit])?;

    TrayIconBuilder::new()
        .tooltip("hex-hub — Running")
        .menu(&menu)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "open_dashboard" => {
                if let Some(window) = app.get_webview_window("main") {
                    let port = hex_hub_core::DEFAULT_PORT;
                    let url: Url = format!("http://127.0.0.1:{}/", port).parse().unwrap();
                    let _ = window.navigate(url);
                    let _ = window.set_focus();
                }
            }
            "open_chat" => {
                if let Some(window) = app.get_webview_window("main") {
                    let port = hex_hub_core::DEFAULT_PORT;
                    let url: Url = format!("http://127.0.0.1:{}/chat", port).parse().unwrap();
                    let _ = window.navigate(url);
                    let _ = window.set_focus();
                }
            }
            "quit" => {
                hex_hub_core::daemon::remove_lock();
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;

    Ok(())
}
