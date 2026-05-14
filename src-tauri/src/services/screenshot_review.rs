use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::{
    domain::settings::{AppSettings, OcrProviderKind},
    infrastructure::screen_capture::{
        self, ScreenCaptureBackend, ScreenCaptureMode, ScreenCaptureRect, ScreenCaptureResult,
    },
    services::ocr::{self, OcrBoundingBox, OcrRequest, OcrResult},
    state::AppState,
};

#[derive(Debug, Clone)]
pub struct ScreenshotReviewSession {
    image_path: PathBuf,
    ocr: ScreenshotReviewOcrPayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenshotReviewCaptureBackend {
    ScreenCaptureKit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenshotReviewCaptureMode {
    Region,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenshotReviewOcrProvider {
    System,
    LlmOcr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenshotReviewOcrStatus {
    Success,
    Empty,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenshotReviewAction {
    TranslateSelectedText,
    CopySelectedText,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotReviewCaptureRectPayload {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotReviewCapturePayload {
    backend: ScreenshotReviewCaptureBackend,
    mode: ScreenshotReviewCaptureMode,
    rect: ScreenshotReviewCaptureRectPayload,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotReviewTextBlockPayload {
    id: String,
    text: String,
    confidence: Option<f32>,
    bounding_box: OcrBoundingBox,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotReviewOcrPayload {
    provider: ScreenshotReviewOcrProvider,
    status: ScreenshotReviewOcrStatus,
    text: String,
    language: Option<String>,
    confidence: Option<f32>,
    error_message: Option<String>,
    blocks: Vec<ScreenshotReviewTextBlockPayload>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenshotReviewRequestedAction {
    Translate,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotReviewPayload {
    session_id: String,
    image_width: u32,
    image_height: u32,
    capture: ScreenshotReviewCapturePayload,
    ocr: ScreenshotReviewOcrPayload,
    requested_action: ScreenshotReviewRequestedAction,
}

impl ScreenshotReviewPayload {
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

impl From<ScreenCaptureBackend> for ScreenshotReviewCaptureBackend {
    fn from(backend: ScreenCaptureBackend) -> Self {
        match backend {
            ScreenCaptureBackend::ScreenCaptureKit => Self::ScreenCaptureKit,
        }
    }
}

impl From<ScreenCaptureMode> for ScreenshotReviewCaptureMode {
    fn from(mode: ScreenCaptureMode) -> Self {
        match mode {
            ScreenCaptureMode::Region => Self::Region,
        }
    }
}

impl From<ScreenCaptureRect> for ScreenshotReviewCaptureRectPayload {
    fn from(rect: ScreenCaptureRect) -> Self {
        Self {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        }
    }
}

type SessionStore = Arc<Mutex<HashMap<String, ScreenshotReviewSession>>>;

fn sessions() -> &'static SessionStore {
    static SESSIONS: OnceLock<SessionStore> = OnceLock::new();
    SESSIONS.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

pub async fn create_session_from_capture(
    state: &AppState,
    settings: &AppSettings,
    capture: ScreenCaptureResult,
) -> Result<ScreenshotReviewPayload> {
    let session_id = next_session_id();
    let image_dimensions = png_dimensions(&capture.image_path).unwrap_or_else(|error| {
        tracing::warn!(
            error = format_args!("{:#}", error),
            image_path = %capture.image_path.display(),
            "failed to read screenshot dimensions for review"
        );
        (0, 0)
    });
    let ocr = recognize_for_review(state, settings, &capture.image_path).await;

    let session = ScreenshotReviewSession {
        image_path: capture.image_path.clone(),
        ocr,
    };
    let payload = build_payload(&session_id, &session, image_dimensions, capture);
    sessions().lock().await.insert(session_id, session);

    Ok(payload)
}

pub async fn get_preview_data_url(session_id: &str) -> Result<String> {
    let image_path = {
        let sessions = sessions().lock().await;
        sessions
            .get(session_id)
            .map(|session| session.image_path.clone())
            .ok_or_else(|| anyhow!("截图 Review 会话不存在或已结束"))?
    };

    encode_image_path_as_data_url(&image_path)
}

pub async fn resolve_confirmed_text(
    session_id: &str,
    edited_text: Option<String>,
) -> Result<String> {
    let sessions = sessions().lock().await;
    let session = sessions
        .get(session_id)
        .ok_or_else(|| anyhow!("截图 Review 会话不存在或已结束"))?;
    let text = edited_text
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| session.ocr.text.clone());
    let text = text.trim().to_string();
    if text.is_empty() {
        bail!("没有可翻译或复制的 OCR 文本，请先编辑文本或重新截图");
    }
    Ok(text)
}

pub async fn remove_session(session_id: &str) -> Result<()> {
    let session = sessions()
        .lock()
        .await
        .remove(session_id)
        .ok_or_else(|| anyhow!("截图 Review 会话不存在或已结束"))?;
    screen_capture::remove_screenshot_file(&session.image_path);
    Ok(())
}

pub async fn session_exists(session_id: &str) -> bool {
    sessions().lock().await.contains_key(session_id)
}

async fn recognize_for_review(
    state: &AppState,
    settings: &AppSettings,
    image_path: &Path,
) -> ScreenshotReviewOcrPayload {
    let provider = provider_payload(settings.ocr.provider.clone());
    let request = OcrRequest {
        image_path: image_path.to_string_lossy().into_owned(),
        focus_point: None,
    };
    let result = match settings.ocr.provider {
        OcrProviderKind::LlmOcr => recognize_with_llm(settings, &request).await,
        OcrProviderKind::System => recognize_with_system(state, request).await,
        OcrProviderKind::Disabled => unreachable!("disabled OCR returned before recognition"),
    };

    match result {
        Ok(result) => ocr_result_payload(provider, result),
        Err(error) => ScreenshotReviewOcrPayload {
            provider,
            status: ScreenshotReviewOcrStatus::Failed,
            text: String::new(),
            language: None,
            confidence: None,
            error_message: Some(error.to_string()),
            blocks: Vec::new(),
        },
    }
}

async fn recognize_with_llm(settings: &AppSettings, request: &OcrRequest) -> Result<OcrResult> {
    let model_id = settings
        .ocr
        .llm_model_id
        .as_deref()
        .context("没有配置 OCR LLM，请先在 AI 功能页选择一个条目")?;
    let binding = settings
        .llm
        .find_model_binding(model_id)
        .with_context(|| format!("OCR LLM 模型不存在: {model_id}"))?;
    ocr::recognize_with_openai_compatible_config(binding, request).await
}

async fn recognize_with_system(state: &AppState, request: OcrRequest) -> Result<OcrResult> {
    let ocr_provider = state.ocr_provider();
    tokio::task::spawn_blocking(move || ocr_provider.recognize(&request))
        .await
        .context("failed to join OCR review task")?
}

fn provider_payload(provider: OcrProviderKind) -> ScreenshotReviewOcrProvider {
    match provider {
        OcrProviderKind::System => ScreenshotReviewOcrProvider::System,
        OcrProviderKind::LlmOcr => ScreenshotReviewOcrProvider::LlmOcr,
        OcrProviderKind::Disabled => unreachable!("screenshot review requires OCR to be enabled"),
    }
}

fn ocr_result_payload(
    provider: ScreenshotReviewOcrProvider,
    result: OcrResult,
) -> ScreenshotReviewOcrPayload {
    let blocks = result
        .blocks
        .into_iter()
        .enumerate()
        .map(|(index, block)| ScreenshotReviewTextBlockPayload {
            id: format!("block-{index}"),
            text: block.text,
            confidence: block.confidence,
            bounding_box: block.bounding_box,
        })
        .collect::<Vec<_>>();
    let text = result.text.trim().to_string();
    let status = if text.is_empty() {
        ScreenshotReviewOcrStatus::Empty
    } else {
        ScreenshotReviewOcrStatus::Success
    };

    ScreenshotReviewOcrPayload {
        provider,
        status,
        text,
        language: result.language,
        confidence: result.confidence,
        error_message: None,
        blocks,
    }
}

fn build_payload(
    session_id: &str,
    session: &ScreenshotReviewSession,
    image_dimensions: (u32, u32),
    capture: ScreenCaptureResult,
) -> ScreenshotReviewPayload {
    ScreenshotReviewPayload {
        session_id: session_id.to_string(),
        image_width: image_dimensions.0,
        image_height: image_dimensions.1,
        capture: ScreenshotReviewCapturePayload {
            backend: capture.backend.into(),
            mode: capture.mode.into(),
            rect: capture.rect.into(),
        },
        ocr: session.ocr.clone(),
        requested_action: ScreenshotReviewRequestedAction::Translate,
    }
}

fn encode_image_path_as_data_url(image_path: &Path) -> Result<String> {
    if !image_path.is_file() {
        bail!("截图文件不存在或已删除");
    }

    let image_bytes = std::fs::read(image_path).with_context(|| "读取截图预览失败")?;
    Ok(format!(
        "data:image/png;base64,{}",
        BASE64_STANDARD.encode(image_bytes)
    ))
}

fn next_session_id() -> String {
    static SESSION_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let sequence = SESSION_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("sr-{timestamp_ms}-{sequence}")
}

fn png_dimensions(path: &Path) -> Result<(u32, u32)> {
    let bytes = std::fs::read(path).with_context(|| "failed to read screenshot header")?;
    if bytes.len() < 24 || &bytes[0..8] != b"\x89PNG\r\n\x1a\n" {
        bail!("screenshot is not a PNG image");
    }

    let width = u32::from_be_bytes(bytes[16..20].try_into()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into()?);
    Ok((width, height))
}
