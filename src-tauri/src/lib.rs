mod audio;
mod capture;
mod commands;
mod config;
mod controls;
#[cfg(debug_assertions)]
mod debug_console;
mod pipeline;
mod preview;
mod session;
mod ui;

use std::sync::Arc;
use std::time::Duration;

use tauri::{Manager, RunEvent, WebviewWindowBuilder, WindowEvent};
use tracing_subscriber::EnvFilter;

use commands::AppState;
use config::ConfigStore;
use session::Msg;

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("camera_settings=info,camera_control=info")),
        )
        .init();

    let gst_error = pipeline::init().err();
    // `--debug` enables the web inspector and the webview's context menu; both would only
    // confuse normal users.
    let debug = std::env::args().skip(1).any(|a| a == "--debug");
    if debug {
        tracing::info!("--debug: web inspector and webview context menu enabled");
    }

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            // The window is declared in tauri.conf.json with `create: false` so devtools can follow `--debug`.
            let window_config = app
                .config()
                .app
                .windows
                .first()
                .cloned()
                .ok_or("no window configured")?;
            WebviewWindowBuilder::from_config(app.handle(), &window_config)?
                .devtools(debug)
                .build()?;

            let config = Arc::new(ConfigStore::load());
            let ui = Arc::new(ui::Ui::new(
                app.handle().clone(),
                &config.get(),
                pipeline::av1_encoder().is_some(),
                debug,
            ));
            if let Some(e) = &gst_error {
                ui.error(e.clone());
            }
            let controls = controls::Controls::spawn(ui.clone(), config.clone());
            let preview = Arc::new(preview::Preview::default());
            let session = session::spawn(ui.clone(), config.clone(), controls.clone(), preview.clone());
            let hp_session = session.clone();
            match camera_control::HotplugMonitor::spawn(move |_| hp_session.send(Msg::Hotplug)) {
                // The monitor thread lives as long as the app.
                Ok(monitor) => std::mem::forget(monitor),
                Err(e) => tracing::warn!("udev monitor unavailable, falling back to periodic rescans: {e}"),
            }
            #[cfg(debug_assertions)]
            debug_console::spawn(app.handle().clone(), session.clone(), controls.clone());
            app.manage(AppState {
                ui,
                config,
                controls,
                preview,
                session,
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, WindowEvent::Resized(_) | WindowEvent::Focused(_)) {
                let minimized = window.is_minimized().unwrap_or(false) || !window.is_visible().unwrap_or(true);
                window.state::<AppState>().session.send(Msg::Minimized(minimized));
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::open_preview,
            commands::preview_ack,
            commands::set_mode,
            commands::set_control,
            commands::reset_control,
            commands::reset_all,
            commands::capture_still,
            commands::start_recording,
            commands::stop_recording,
            commands::set_prefs,
            commands::reveal,
            commands::open_capture,
            commands::set_hidden,
            commands::set_paused,
            commands::quit,
        ])
        .build(tauri::generate_context!())
        .expect("error while building the app");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            let (tx, rx) = std::sync::mpsc::channel();
            app.state::<AppState>().session.send(Msg::Shutdown(tx));
            // Bounded by the session's own finalize (10 s) and teardown (1 s) timeouts.
            let _ = rx.recv_timeout(Duration::from_secs(12));
        }
    });
}
