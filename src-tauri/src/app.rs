use anyhow::{Context, Result};
use tauri::{Manager, Wry};
use tauri_plugin_global_shortcut::{Builder as GlobalShortcutBuilder, ShortcutState};
use time::{format_description::well_known::Rfc3339, UtcOffset};
use tokio::{task, time as tokio_time};
use tracing_subscriber::fmt::time::OffsetTime;
use tracing_subscriber::EnvFilter;

use crate::{
    commands,
    infrastructure::{hotkey, window},
    services::{
        application::APPLICATION_CACHE_REFRESH_INTERVAL, executor::ExecutorService,
        matcher::MatcherService, ocr, selection,
    },
    state::{AppState, ShortcutAction, ShortcutRuntimeState},
};

pub fn run() -> Result<()> {
    init_tracing();
    tracing::info!(
        version = env!("WABITY_APP_VERSION"),
        build_date = env!("WABITY_BUILD_DATE"),
        "starting wabity"
    );

    let shortcut_state = ShortcutRuntimeState::default();
    let shortcut_for_handler = shortcut_state.clone();

    let builder = tauri::Builder::<Wry>::default();

    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());

    let builder = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            GlobalShortcutBuilder::new()
                .with_handler(move |_app, shortcut, event| {
                    let Some(action) = shortcut_for_handler.shortcut_action(*shortcut) else {
                        return;
                    };

                    match event.state() {
                        ShortcutState::Pressed => {
                            if !shortcut_for_handler.begin_shortcut_press(action) {
                                return;
                            }

                            match action {
                                ShortcutAction::ToggleLauncher => {
                                    let selected_text = selection::get_selected_text()
                                        .inspect_err(|error| {
                                            tracing::warn!(?error, "failed to get selected text")
                                        })
                                        .ok()
                                        .flatten();

                                    if let Err(error) =
                                        window::toggle_main_window(_app, selected_text)
                                    {
                                        tracing::error!(
                                            ?error,
                                            "failed to toggle main window from shortcut"
                                        );
                                    }
                                }
                                ShortcutAction::OcrCapture => {
                                    if !shortcut_for_handler.begin_ocr_capture() {
                                        return;
                                    }

                                    let app_handle = _app.clone();
                                    let shortcut_state = shortcut_for_handler.clone();
                                    tauri::async_runtime::spawn(async move {
                                        let flow_result =
                                            handle_ocr_shortcut(app_handle.clone()).await;
                                        shortcut_state.end_ocr_capture();

                                        if let Err(error) = flow_result {
                                            tracing::error!(
                                                ?error,
                                                "failed to complete OCR capture shortcut"
                                            );
                                            if let Err(show_error) =
                                                window::show_main_window_with_error(
                                                    &app_handle,
                                                    &format!("OCR 失败：{error}"),
                                                )
                                            {
                                                tracing::error!(
                                                    ?show_error,
                                                    "failed to show launcher after OCR error"
                                                );
                                            }
                                        }
                                    });
                                }
                            }
                        }
                        ShortcutState::Released => {
                            shortcut_for_handler.end_shortcut_press(action);
                        }
                    }
                })
                .build(),
        );
    let builder = attach_invoke_handler(builder);

    builder
        .setup(move |app| {
            let app_state = tauri::async_runtime::block_on(AppState::new(
                MatcherService::new(),
                ExecutorService::new(),
            ))?;

            app.manage(app_state.clone());
            app.manage(shortcut_state.clone());
            app_state.acp().start_event_loop();
            tauri::async_runtime::block_on(app_state.restore_acp_sessions())?;
            start_application_cache_tasks(app_state.application().clone());

            let main_window = app
                .get_webview_window("main")
                .context("main window must exist")?;

            window::configure_main_window(&main_window)?;

            // Register shortcut from config
            tauri::async_runtime::block_on(async {
                let config = app_state.app_config().await?;
                let shortcut = match hotkey::parse_shortcut(&config.shortcuts.toggle_launcher) {
                    Some(shortcut) => shortcut,
                    None => {
                        let default_shortcut =
                            crate::infrastructure::config::ShortcutConfig::default();
                        tracing::warn!(
                            invalid_shortcut = %config.shortcuts.toggle_launcher,
                            fallback_shortcut = %default_shortcut.toggle_launcher,
                            "invalid shortcut in config, falling back to default"
                        );
                        app_state
                            .update_shortcut("toggle_launcher", &default_shortcut.toggle_launcher)
                            .await?;
                        hotkey::parse_shortcut(&default_shortcut.toggle_launcher)
                            .context("default launcher shortcut must be valid")?
                    }
                };

                hotkey::register_shortcut(app.handle(), shortcut)?;
                shortcut_state.set_launcher_shortcut(Some(shortcut));

                let ocr_shortcut = match hotkey::parse_shortcut(&config.shortcuts.ocr_capture) {
                    Some(shortcut) => shortcut,
                    None => {
                        let default_shortcut =
                            crate::infrastructure::config::ShortcutConfig::default();
                        tracing::warn!(
                            invalid_shortcut = %config.shortcuts.ocr_capture,
                            fallback_shortcut = %default_shortcut.ocr_capture,
                            "invalid OCR shortcut in config, falling back to default"
                        );
                        app_state
                            .update_shortcut("ocr_capture", &default_shortcut.ocr_capture)
                            .await?;
                        hotkey::parse_shortcut(&default_shortcut.ocr_capture)
                            .context("default OCR shortcut must be valid")?
                    }
                };

                match hotkey::register_shortcut(app.handle(), ocr_shortcut) {
                    Ok(()) => shortcut_state.set_ocr_shortcut(Some(ocr_shortcut)),
                    Err(error) => {
                        tracing::warn!(
                            ?error,
                            configured_shortcut = %config.shortcuts.ocr_capture,
                            "failed to register OCR capture shortcut"
                        );
                        shortcut_state.set_ocr_shortcut(None);
                    }
                }
                Ok::<(), anyhow::Error>(())
            })?;

            main_window
                .hide()
                .context("failed to hide launcher on startup")?;
            shortcut_state.set_launcher_visible(false);

            Ok(())
        })
        .run(application_context())
        .context("error while running tauri application")?;

    Ok(())
}

#[cfg(rust_analyzer)]
fn application_context() -> tauri::Context<Wry> {
    // Keep rust-analyzer on a macro-free path because Tauri context generation currently triggers false hard errors.
    tauri::test::mock_context(tauri::test::noop_assets())
}

#[cfg(not(rust_analyzer))]
fn application_context() -> tauri::Context<Wry> {
    tauri::generate_context!("tauri.conf.json")
}

fn attach_invoke_handler(builder: tauri::Builder<Wry>) -> tauri::Builder<Wry> {
    builder.invoke_handler(commands::handle_invoke)
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let local_offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    let timer = OffsetTime::new(local_offset, Rfc3339);

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_timer(timer)
        .try_init();
}

async fn handle_ocr_shortcut(app: tauri::AppHandle) -> Result<()> {
    let ocr_provider = {
        let state = app.state::<AppState>();
        state.ocr_provider()
    };

    if app.state::<ShortcutRuntimeState>().is_launcher_visible() {
        window::hide_main_window(&app)?;
    }

    let ocr_result = task::spawn_blocking(move || -> Result<Option<ocr::OcrResult>> {
        let screenshot_path = match ocr::capture_interactive_screenshot()? {
            Some(path) => path,
            None => return Ok(None),
        };

        let request = ocr::OcrRequest {
            image_path: screenshot_path.to_string_lossy().into_owned(),
            focus_point: None,
        };
        let result = ocr_provider.recognize(&request);
        ocr::remove_screenshot_file(&screenshot_path);
        result.map(Some)
    })
    .await
    .context("failed to join OCR capture task")??;

    let Some(ocr_result) = ocr_result else {
        return Ok(());
    };

    if ocr_result.text.trim().is_empty() {
        window::show_main_window_with_error(&app, "OCR 未识别到可用文本")?;
        return Ok(());
    }

    window::show_main_window_with_text(&app, ocr_result.text)?;
    Ok(())
}

fn start_application_cache_tasks(application: crate::services::application::ApplicationService) {
    tauri::async_runtime::spawn(async move {
        if let Err(error) = application.refresh_now().await {
            tracing::warn!(?error, "failed to warm application cache on startup");
        }

        let mut interval = tokio_time::interval(APPLICATION_CACHE_REFRESH_INTERVAL);
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Err(error) = application.refresh_now().await {
                tracing::warn!(?error, "failed to refresh application cache on interval");
            }
        }
    });
}
