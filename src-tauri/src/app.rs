use anyhow::{Context, Result};
use std::path::PathBuf;
use tauri::{Manager, Wry};
#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_global_shortcut::{Builder as GlobalShortcutBuilder, ShortcutState};
use time::{format_description::well_known::Rfc3339, UtcOffset};
use tokio::{task, time as tokio_time};
use tracing_subscriber::fmt::time::OffsetTime;
use tracing_subscriber::EnvFilter;

use crate::{
    commands,
    infrastructure::{
        autostart,
        config::{ShortcutConfig, ShortcutKey},
        hotkey, window,
    },
    services::{
        application::APPLICATION_CACHE_REFRESH_INTERVAL, executor::ExecutorService,
        matcher::MatcherService, ocr, process::PROCESS_CACHE_REFRESH_INTERVAL, selection,
        translate,
    },
    state::{AppState, ShortcutAction, ShortcutRuntimeState},
};

#[cfg(target_os = "macos")]
use tauri::ActivationPolicy;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShortcutTranslationSource {
    mode: window::ShortcutTranslationSourceMode,
    text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ShortcutWindowVisibility {
    launcher_visible: bool,
    clipboard_history_visible: bool,
    clipboard_selection_mode: Option<window::ClipboardHistorySelectionMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShortcutWindowRestoreTarget {
    Launcher,
    ClipboardHistory,
}

impl ShortcutWindowVisibility {
    fn from_state(shortcut_state: &ShortcutRuntimeState) -> Self {
        let clipboard_history_visible = shortcut_state.is_clipboard_history_visible();
        let clipboard_selection_mode = clipboard_history_visible.then_some(
            if shortcut_state.clipboard_window_preserves_launcher_focus() {
                window::ClipboardHistorySelectionMode::InsertIntoLauncher
            } else {
                window::ClipboardHistorySelectionMode::PasteExternally
            },
        );

        Self {
            launcher_visible: shortcut_state.is_launcher_visible(),
            clipboard_history_visible,
            clipboard_selection_mode,
        }
    }

    fn restore_target(self) -> Option<ShortcutWindowRestoreTarget> {
        if self.clipboard_history_visible {
            Some(ShortcutWindowRestoreTarget::ClipboardHistory)
        } else if self.launcher_visible {
            Some(ShortcutWindowRestoreTarget::Launcher)
        } else {
            None
        }
    }
}

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
fn handle_secondary_launch(app: &tauri::AppHandle<Wry>, argv_count: usize) {
    tracing::info!(
        argv_count,
        "detected secondary launch and redirecting to the existing instance"
    );
    if let Err(error) = window::reveal_main_window(app) {
        tracing::error!(
            ?error,
            "failed to reveal existing instance after secondary launch"
        );
    }
}

#[cfg(target_os = "macos")]
fn macos_activation_policy(show_in_dock: bool) -> ActivationPolicy {
    if show_in_dock {
        ActivationPolicy::Regular
    } else {
        ActivationPolicy::Accessory
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn sync_macos_app_visibility(app: &tauri::AppHandle, show_in_dock: bool) -> Result<()> {
    app.set_activation_policy(macos_activation_policy(show_in_dock))
        .with_context(|| {
            format!("failed to set macOS activation policy for show_in_dock={show_in_dock}")
        })?;
    app.set_dock_visibility(show_in_dock)
        .with_context(|| format!("failed to set macOS Dock visibility to {show_in_dock}"))?;
    Ok(())
}

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

    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
        handle_secondary_launch(app, argv.len());
    }));

    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());

    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    let builder = builder.plugin(tauri_plugin_autostart::init(
        MacosLauncher::LaunchAgent,
        None,
    ));

    let builder = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
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
                                tracing::info!(
                                    action = shortcut_action_name(action),
                                    "ignoring shortcut press because action is still marked as pressed"
                                );
                                return;
                            }

                            tracing::info!(
                                action = shortcut_action_name(action),
                                "accepted global shortcut press"
                            );

                            match action {
                                ShortcutAction::ToggleLauncher => {
                                    if let Err(error) = window::toggle_main_window(_app) {
                                        tracing::error!(
                                            ?error,
                                            "failed to toggle main window from shortcut"
                                        );
                                    }
                                }
                                ShortcutAction::OcrTranslate => {
                                    if !shortcut_for_handler.begin_ocr_capture() {
                                        return;
                                    }

                                    // On macOS, simulated copy must stay on the shortcut handler
                                    // thread. Running enigo from a Tokio worker crashes inside
                                    // HIToolbox input-source lookup.
                                    let selected_text = selection::get_selected_text()
                                        .inspect_err(|error| {
                                            tracing::warn!(
                                                ?error,
                                                "failed to get selected text for shortcut"
                                            )
                                        })
                                        .ok()
                                        .flatten();
                                    let app_handle = _app.clone();
                                    let shortcut_state = shortcut_for_handler.clone();
                                    tauri::async_runtime::spawn(async move {
                                        let flow_result = handle_ocr_translate_shortcut(
                                            app_handle.clone(),
                                            selected_text,
                                        )
                                        .await;
                                        shortcut_state.end_ocr_capture();

                                        if let Err(error) = flow_result {
                                            tracing::error!(
                                                ?error,
                                                "failed to complete translate shortcut"
                                            );
                                            if shortcut_state.is_clipboard_history_visible() {
                                                tracing::debug!(
                                                    "restored clipboard history after OCR shortcut failure; skipping launcher error window"
                                                );
                                                return;
                                            }
                                            if let Err(show_error) =
                                                window::show_main_window_with_error(
                                                    &app_handle,
                                                    &format!("翻译失败：{error}"),
                                                )
                                            {
                                                tracing::error!(
                                                    ?show_error,
                                                    "failed to show launcher after translate shortcut error"
                                                );
                                            }
                                        }
                                    });
                                }
                                ShortcutAction::OpenClipboardHistory => {
                                    if let Err(error) =
                                        window::show_main_window_with_clipboard_history_panel(_app)
                                    {
                                        tracing::error!(
                                            ?error,
                                            "failed to show clipboard history panel from shortcut"
                                        );
                                    }
                                }
                            }
                        }
                        ShortcutState::Released => {
                            tracing::debug!(
                                action = shortcut_action_name(action),
                                "received global shortcut release"
                            );
                            shortcut_for_handler.end_shortcut_press(action);
                        }
                    }
                })
                .build(),
        );
    let builder = builder.invoke_handler(commands::handle_invoke);

    builder
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let app_state = tauri::async_runtime::block_on(AppState::new(
                app_handle,
                shortcut_state.clone(),
                MatcherService::new(),
                ExecutorService::new(),
            ))?;

            #[cfg(target_os = "macos")]
            {
                let show_in_dock = tauri::async_runtime::block_on(app_state.app_config())?
                    .general
                    .show_in_dock;
                sync_macos_app_visibility(app.handle(), show_in_dock)?;
            }

            app.manage(app_state.clone());
            app.manage(shortcut_state.clone());
            reconcile_configured_autostart(&app.handle().clone(), &app_state);
            app_state.acp().start_event_loop();
            tauri::async_runtime::block_on(app_state.restore_acp_sessions())?;
            start_application_cache_tasks(app_state.application().clone());
            start_process_cache_tasks(app_state.process().clone());

            let main_window = app
                .get_webview_window("main")
                .context("main window must exist")?;
            let clipboard_window = app
                .get_webview_window("clipboard")
                .context("clipboard window must exist")?;

            window::configure_main_window(&main_window)?;
            window::configure_clipboard_window(&clipboard_window)?;

            let shortcut_failures = tauri::async_runtime::block_on(initialize_shortcuts(
                &app.handle().clone(),
                &app_state,
                &shortcut_state,
            ))?;

            main_window
                .hide()
                .context("failed to hide launcher on startup")?;
            clipboard_window
                .hide()
                .context("failed to hide clipboard history window on startup")?;
            shortcut_state.set_launcher_visible(false);
            shortcut_state.set_clipboard_history_visible(false);
            commands::launcher::emit_shortcut_runtime_status(app.handle(), &shortcut_state)?;

            if !shortcut_failures.is_empty() {
                let launcher_failure = shortcut_failures
                    .iter()
                    .find(|failure| failure.key == ShortcutKey::ToggleLauncher);
                if launcher_failure.is_some() {
                    window::reveal_main_window(app.handle())?;
                }

                let shortcut_summary = shortcut_failures
                    .iter()
                    .map(|failure| failure.message.as_str())
                    .collect::<Vec<_>>()
                    .join("；");
                tracing::warn!(shortcut_summary, "startup shortcut registration degraded");
            }

            Ok(())
        })
        .run(application_context())
        .context("error while running tauri application")?;

    Ok(())
}

fn reconcile_configured_autostart(app: &tauri::AppHandle, app_state: &AppState) {
    let configured_enabled = match tauri::async_runtime::block_on(app_state.app_config()) {
        Ok(config) => config.general.auto_start,
        Err(error) => {
            tracing::warn!(?error, "failed to load config for autostart reconciliation");
            return;
        }
    };

    // Startup reconciliation is best-effort because login-item drift should not block launcher startup.
    if let Err(error) = autostart::sync_autostart(app, configured_enabled) {
        tracing::warn!(
            ?error,
            enabled = configured_enabled,
            "failed to reconcile autostart state on startup"
        );
    }
}

fn start_process_cache_tasks(process: crate::services::process::ProcessService) {
    tauri::async_runtime::spawn(async move {
        if let Err(error) = process.refresh_now().await {
            tracing::warn!(?error, "failed to prewarm process cache");
        }

        let mut interval = tokio_time::interval(PROCESS_CACHE_REFRESH_INTERVAL);
        interval.tick().await;

        loop {
            interval.tick().await;
            if let Err(error) = process.refresh_now().await {
                tracing::warn!(?error, "failed to refresh process cache on interval");
            }
        }
    });
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

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let local_offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    let timer = OffsetTime::new(local_offset, Rfc3339);

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_timer(timer)
        .try_init();
}

fn shortcut_action_name(action: ShortcutAction) -> &'static str {
    match action {
        ShortcutAction::ToggleLauncher => "toggle_launcher",
        ShortcutAction::OcrTranslate => "ocr_translate",
        ShortcutAction::OpenClipboardHistory => "open_clipboard_history",
    }
}

#[derive(Debug, Clone, Copy)]
enum ShortcutRegistrationMode {
    Required,
    Optional,
}

#[derive(Debug, Clone)]
struct ShortcutRegistrationFailure {
    key: ShortcutKey,
    message: String,
}

async fn initialize_shortcuts(
    app: &tauri::AppHandle,
    app_state: &AppState,
    shortcut_state: &ShortcutRuntimeState,
) -> Result<Vec<ShortcutRegistrationFailure>> {
    let config = app_state.app_config().await?;
    let mut failures = Vec::new();

    if let Some(failure) = register_startup_shortcut(
        app,
        app_state,
        shortcut_state,
        &config.shortcuts,
        ShortcutKey::ToggleLauncher,
        ShortcutRegistrationMode::Required,
    )
    .await?
    {
        failures.push(failure);
    }
    if let Some(failure) = register_startup_shortcut(
        app,
        app_state,
        shortcut_state,
        &config.shortcuts,
        ShortcutKey::OcrTranslate,
        ShortcutRegistrationMode::Optional,
    )
    .await?
    {
        failures.push(failure);
    }
    if let Some(failure) = register_startup_shortcut(
        app,
        app_state,
        shortcut_state,
        &config.shortcuts,
        ShortcutKey::OpenClipboardHistory,
        ShortcutRegistrationMode::Optional,
    )
    .await?
    {
        failures.push(failure);
    }

    Ok(failures)
}

async fn register_startup_shortcut(
    app: &tauri::AppHandle,
    app_state: &AppState,
    shortcut_state: &ShortcutRuntimeState,
    shortcuts: &ShortcutConfig,
    key: ShortcutKey,
    mode: ShortcutRegistrationMode,
) -> Result<Option<ShortcutRegistrationFailure>> {
    let (shortcut, configured_value) =
        resolve_configured_shortcut(app_state, shortcuts, key).await?;

    match hotkey::register_shortcut(app, shortcut) {
        Ok(()) => {
            shortcut_state.set_shortcut(key, Some(shortcut));
            shortcut_state.set_shortcut_registration_status(key, configured_value, true, None);
            Ok(None)
        }
        Err(error) => match mode {
            ShortcutRegistrationMode::Required => {
                let message = format!(
                    "无法注册快捷键 {}（{}）：{}",
                    key.display_name(),
                    configured_value,
                    error
                );
                tracing::error!(
                    ?error,
                    shortcut_key = %key.as_str(),
                    configured_shortcut = %configured_value,
                    "failed to register required shortcut"
                );
                shortcut_state.set_shortcut(key, None);
                shortcut_state.set_shortcut_registration_status(
                    key,
                    configured_value.clone(),
                    false,
                    Some(message.clone()),
                );
                Ok(Some(ShortcutRegistrationFailure { key, message }))
            }
            ShortcutRegistrationMode::Optional => {
                shortcut_state.set_shortcut(key, None);
                let message = format!(
                    "无法注册快捷键 {}（{}）：{}",
                    key.display_name(),
                    configured_value,
                    error
                );
                tracing::warn!(
                    ?error,
                    shortcut_key = %key.as_str(),
                    configured_shortcut = %configured_value,
                    "failed to register optional shortcut"
                );
                shortcut_state.set_shortcut_registration_status(
                    key,
                    configured_value.clone(),
                    false,
                    Some(message.clone()),
                );
                Ok(Some(ShortcutRegistrationFailure { key, message }))
            }
        },
    }
}

async fn resolve_configured_shortcut(
    app_state: &AppState,
    shortcuts: &ShortcutConfig,
    key: ShortcutKey,
) -> Result<(tauri_plugin_global_shortcut::Shortcut, String)> {
    let configured_value = shortcuts.get(key);
    if let Some(shortcut) = hotkey::parse_shortcut(configured_value) {
        return Ok((shortcut, configured_value.to_string()));
    }

    let fallback_value = ShortcutConfig::default_value(key);
    tracing::warn!(
        shortcut_key = %key.as_str(),
        invalid_shortcut = %configured_value,
        fallback_shortcut = %fallback_value,
        "invalid shortcut in config, falling back to default"
    );
    app_state.update_shortcut(key, fallback_value).await?;

    let fallback_shortcut = hotkey::parse_shortcut(fallback_value)
        .with_context(|| format!("default {} shortcut must be valid", key.display_name()))?;
    Ok((fallback_shortcut, fallback_value.to_string()))
}

async fn handle_ocr_translate_shortcut(
    app: tauri::AppHandle,
    selected_text: Option<String>,
) -> Result<()> {
    let translation_source =
        if let Some(source) = resolve_shortcut_translation_source(selected_text, None) {
            source
        } else {
            let ocr_text = capture_ocr_text(app.clone()).await?;
            let Some(source) = resolve_shortcut_translation_source(None, ocr_text) else {
                return Ok(());
            };
            source
        };

    window::show_main_window_with_shortcut_translation_started(
        &app,
        translation_source.mode,
        translation_source.text.clone(),
    )?;

    let settings = app.state::<AppState>().app_settings().await?;
    let source_text = translation_source.text.clone();
    let source_mode = translation_source.mode;
    let mut partial_text = String::new();
    let translation_result = translate::execute_translation_with_callbacks(
        &source_text,
        &settings.prompts,
        &settings.llm,
        translate::TranslationCallbacks {
            on_text_delta: Some(&mut |delta: &str| {
                partial_text.push_str(delta);
                let _ = window::emit_shortcut_translation_stream(
                    &app,
                    source_mode,
                    source_text.clone(),
                    partial_text.clone(),
                );
            }),
        },
    )
    .await?;

    window::emit_shortcut_translation_result(
        &app,
        translation_source.mode,
        translation_source.text,
        translation_result,
    )?;
    Ok(())
}

fn resolve_shortcut_translation_source(
    selected_text: Option<String>,
    ocr_text: Option<String>,
) -> Option<ShortcutTranslationSource> {
    selected_text
        .filter(|text| !text.trim().is_empty())
        .map(|text| ShortcutTranslationSource {
            mode: window::ShortcutTranslationSourceMode::Selection,
            text,
        })
        .or_else(|| {
            ocr_text
                .filter(|text| !text.trim().is_empty())
                .map(|text| ShortcutTranslationSource {
                    mode: window::ShortcutTranslationSourceMode::Ocr,
                    text,
                })
        })
}

async fn capture_ocr_text(app: tauri::AppHandle) -> Result<Option<String>> {
    let settings = app.state::<AppState>().app_settings().await?;
    let hidden_windows = hide_visible_shortcut_windows(&app)?;

    let screenshot_path = capture_ocr_screenshot_path().await?;
    let Some(screenshot_path) = screenshot_path else {
        restore_hidden_shortcut_windows(&app, hidden_windows)?;
        return Ok(None);
    };

    let request = ocr::OcrRequest {
        image_path: screenshot_path.to_string_lossy().into_owned(),
        focus_point: None,
    };
    let ocr_result = match perform_shortcut_ocr(&app, &settings, request, screenshot_path).await {
        Ok(result) => result,
        Err(error) => {
            if should_restore_shortcut_windows_after_ocr_error(hidden_windows) {
                restore_hidden_shortcut_windows(&app, hidden_windows)?;
                window::emit_clipboard_history_panel_error(&app, &format!("翻译失败：{error}"))?;
                return Ok(None);
            }
            return Err(error);
        }
    };

    let Some(ocr_result) = ocr_result else {
        restore_hidden_shortcut_windows(&app, hidden_windows)?;
        return Ok(None);
    };

    if ocr_result.text.trim().is_empty() {
        if should_show_shortcut_ocr_empty_text_error_in_main_window(hidden_windows) {
            window::show_main_window_with_error(&app, "OCR 未识别到可用文本")?;
        } else {
            restore_hidden_shortcut_windows(&app, hidden_windows)?;
            window::emit_clipboard_history_panel_error(&app, "OCR 未识别到可用文本")?;
        }
        return Ok(None);
    }

    Ok(Some(ocr_result.text))
}

fn hide_visible_shortcut_windows(app: &tauri::AppHandle) -> Result<ShortcutWindowVisibility> {
    let shortcut_state = app.state::<ShortcutRuntimeState>();
    let visibility = ShortcutWindowVisibility::from_state(shortcut_state.inner());

    if visibility.launcher_visible {
        window::hide_main_window(app)?;
    }

    if visibility.clipboard_history_visible {
        window::hide_launcher_window(app, Some("clipboard"))?;
    }

    Ok(visibility)
}

fn restore_hidden_shortcut_windows(
    app: &tauri::AppHandle,
    visibility: ShortcutWindowVisibility,
) -> Result<()> {
    match visibility.restore_target() {
        Some(ShortcutWindowRestoreTarget::ClipboardHistory) => {
            window::show_clipboard_history_panel(
                app,
                visibility
                    .clipboard_selection_mode
                    .unwrap_or(window::ClipboardHistorySelectionMode::PasteExternally),
            )
        }
        Some(ShortcutWindowRestoreTarget::Launcher) => window::reveal_main_window(app),
        None => Ok(()),
    }
}

fn should_restore_shortcut_windows_after_ocr_error(visibility: ShortcutWindowVisibility) -> bool {
    visibility.restore_target() == Some(ShortcutWindowRestoreTarget::ClipboardHistory)
}

fn should_show_shortcut_ocr_empty_text_error_in_main_window(
    visibility: ShortcutWindowVisibility,
) -> bool {
    visibility.restore_target() != Some(ShortcutWindowRestoreTarget::ClipboardHistory)
}

async fn capture_ocr_screenshot_path() -> Result<Option<PathBuf>> {
    task::spawn_blocking(ocr::capture_interactive_screenshot)
        .await
        .context("failed to join OCR screenshot task")?
}

async fn perform_shortcut_ocr(
    app: &tauri::AppHandle,
    settings: &crate::domain::settings::AppSettings,
    request: ocr::OcrRequest,
    screenshot_path: PathBuf,
) -> Result<Option<ocr::OcrResult>> {
    match settings.ocr.provider {
        crate::domain::settings::OcrProviderKind::LlmOcr => {
            let result = async {
                let provider_id = settings
                    .ocr
                    .llm_provider_id
                    .as_deref()
                    .context("没有配置 OCR LLM，请先在 AI 功能页选择一个条目")?;
                let provider = settings
                    .llm
                    .providers
                    .iter()
                    .find(|provider| provider.id == provider_id)
                    .with_context(|| format!("OCR LLM provider 不存在: {provider_id}"))?;
                ocr::recognize_with_openai_compatible_config(provider, &request).await
            }
            .await;
            ocr::remove_screenshot_file(&screenshot_path);
            result.map(Some)
        }
        _ => {
            let ocr_provider = {
                let state = app.state::<AppState>();
                state.ocr_provider()
            };
            task::spawn_blocking(move || {
                let result = ocr_provider.recognize(&request);
                ocr::remove_screenshot_file(&screenshot_path);
                result.map(Some)
            })
            .await
            .context("failed to join OCR capture task")?
        }
    }
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

#[cfg(test)]
mod tests {
    use super::{
        resolve_shortcut_translation_source, should_restore_shortcut_windows_after_ocr_error,
        should_show_shortcut_ocr_empty_text_error_in_main_window, ShortcutWindowRestoreTarget,
        ShortcutWindowVisibility,
    };
    use crate::infrastructure::window::{
        ClipboardHistorySelectionMode, ShortcutTranslationSourceMode,
    };
    use crate::state::ShortcutRuntimeState;
    #[cfg(target_os = "macos")]
    use tauri::ActivationPolicy;

    #[test]
    fn shortcut_translation_prefers_selected_text() {
        let source = resolve_shortcut_translation_source(
            Some("selected".to_string()),
            Some("ocr".to_string()),
        )
        .expect("selection should win");

        assert_eq!(source.mode, ShortcutTranslationSourceMode::Selection);
        assert_eq!(source.text, "selected");
    }

    #[test]
    fn shortcut_translation_falls_back_to_ocr_text() {
        let source = resolve_shortcut_translation_source(None, Some("ocr".to_string()))
            .expect("ocr should be used when selection is absent");

        assert_eq!(source.mode, ShortcutTranslationSourceMode::Ocr);
        assert_eq!(source.text, "ocr");
    }

    #[test]
    fn shortcut_translation_rejects_blank_sources() {
        assert!(resolve_shortcut_translation_source(
            Some("   ".to_string()),
            Some("\n".to_string()),
        )
        .is_none());
    }

    #[test]
    fn shortcut_window_restore_prefers_clipboard_history() {
        let visibility = ShortcutWindowVisibility {
            launcher_visible: true,
            clipboard_history_visible: true,
            clipboard_selection_mode: Some(ClipboardHistorySelectionMode::InsertIntoLauncher),
        };

        assert_eq!(
            visibility.restore_target(),
            Some(ShortcutWindowRestoreTarget::ClipboardHistory)
        );
    }

    #[test]
    fn shortcut_window_restore_uses_launcher_when_only_launcher_was_visible() {
        let visibility = ShortcutWindowVisibility {
            launcher_visible: true,
            clipboard_history_visible: false,
            clipboard_selection_mode: None,
        };

        assert_eq!(
            visibility.restore_target(),
            Some(ShortcutWindowRestoreTarget::Launcher)
        );
    }

    #[test]
    fn shortcut_window_restore_skips_when_no_window_was_visible() {
        let visibility = ShortcutWindowVisibility {
            launcher_visible: false,
            clipboard_history_visible: false,
            clipboard_selection_mode: None,
        };

        assert_eq!(visibility.restore_target(), None);
    }

    #[test]
    fn shortcut_ocr_error_restores_clipboard_history_panel() {
        let visibility = ShortcutWindowVisibility {
            launcher_visible: true,
            clipboard_history_visible: true,
            clipboard_selection_mode: Some(ClipboardHistorySelectionMode::InsertIntoLauncher),
        };

        assert!(should_restore_shortcut_windows_after_ocr_error(visibility));
    }

    #[test]
    fn shortcut_ocr_error_keeps_launcher_error_path_when_clipboard_was_not_visible() {
        let visibility = ShortcutWindowVisibility {
            launcher_visible: true,
            clipboard_history_visible: false,
            clipboard_selection_mode: None,
        };

        assert!(!should_restore_shortcut_windows_after_ocr_error(visibility));
    }

    #[test]
    fn blank_ocr_result_restores_clipboard_history_instead_of_switching_to_main_window() {
        let visibility = ShortcutWindowVisibility {
            launcher_visible: true,
            clipboard_history_visible: true,
            clipboard_selection_mode: Some(ClipboardHistorySelectionMode::PasteExternally),
        };

        assert!(!should_show_shortcut_ocr_empty_text_error_in_main_window(
            visibility
        ));
    }

    #[test]
    fn blank_ocr_result_uses_main_window_error_without_clipboard_history() {
        let visibility = ShortcutWindowVisibility {
            launcher_visible: true,
            clipboard_history_visible: false,
            clipboard_selection_mode: None,
        };

        assert!(should_show_shortcut_ocr_empty_text_error_in_main_window(
            visibility
        ));
    }

    #[test]
    fn shortcut_window_visibility_preserves_clipboard_insert_mode() {
        let state = ShortcutRuntimeState::default();
        state.set_clipboard_history_visible(true);
        state.set_clipboard_window_preserves_launcher_focus(true);

        let visibility = ShortcutWindowVisibility::from_state(&state);

        assert_eq!(
            visibility.clipboard_selection_mode,
            Some(ClipboardHistorySelectionMode::InsertIntoLauncher)
        );
        assert_eq!(
            visibility.restore_target(),
            Some(ShortcutWindowRestoreTarget::ClipboardHistory)
        );
    }

    #[test]
    fn shortcut_window_visibility_preserves_clipboard_external_mode() {
        let state = ShortcutRuntimeState::default();
        state.set_clipboard_history_visible(true);
        state.set_clipboard_window_preserves_launcher_focus(false);

        let visibility = ShortcutWindowVisibility::from_state(&state);

        assert_eq!(
            visibility.clipboard_selection_mode,
            Some(ClipboardHistorySelectionMode::PasteExternally)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_activation_policy_tracks_show_in_dock_setting() {
        assert!(matches!(
            super::macos_activation_policy(true),
            ActivationPolicy::Regular
        ));
        assert!(matches!(
            super::macos_activation_policy(false),
            ActivationPolicy::Accessory
        ));
    }
}
