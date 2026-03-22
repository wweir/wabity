use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::settings::LlmProviderConfig;
use crate::infrastructure::openai_compatible::{
    extract_provider_error_message, extract_responses_text, normalize_base_url,
    parse_json_or_sse_payload,
};

const OPENAI_COMPATIBLE_OCR_PROMPT: &str =
    "Extract all readable text from this image. Return only the extracted text and preserve line breaks. If there is no readable text, return an empty string.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrBoundingBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrTextBlock {
    pub text: String,
    pub confidence: Option<f32>,
    pub bounding_box: OcrBoundingBox,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrRequest {
    pub image_path: String,
    pub focus_point: Option<OcrPoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrResult {
    pub text: String,
    pub language: Option<String>,
    pub confidence: Option<f32>,
    #[serde(default)]
    pub blocks: Vec<OcrTextBlock>,
    pub matched_block: Option<OcrTextBlock>,
}

pub trait OcrProvider: Send + Sync {
    fn recognize(&self, request: &OcrRequest) -> Result<OcrResult>;
}

pub struct UnavailableOcrProvider {
    reason: String,
}

impl UnavailableOcrProvider {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl OcrProvider for UnavailableOcrProvider {
    fn recognize(&self, request: &OcrRequest) -> Result<OcrResult> {
        bail!("{}: {}", self.reason, request.image_path)
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug, Default)]
pub struct MacOsVisionOcrProvider;

#[cfg(target_os = "macos")]
impl OcrProvider for MacOsVisionOcrProvider {
    fn recognize(&self, request: &OcrRequest) -> Result<OcrResult> {
        recognize_with_macos_vision(request)
    }
}

pub struct OpenAiCompatibleOcrProvider {
    base_url: String,
    api_key: String,
    model: String,
    client: reqwest::blocking::Client,
}

impl OpenAiCompatibleOcrProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Result<Self> {
        let base_url = normalize_base_url(&base_url.into(), "OpenAI-compatible base URL")?;
        let api_key = api_key.into().trim().to_string();
        let model = model.into().trim().to_string();
        if model.is_empty() {
            bail!("OpenAI-compatible OCR model must not be empty");
        }

        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(45))
            .build()
            .context("failed to build HTTP client for OpenAI-compatible OCR")?;

        Ok(Self {
            base_url,
            api_key,
            model,
            client,
        })
    }

    pub fn from_config(config: &LlmProviderConfig) -> Result<Self> {
        Self::new(
            config.base_url.clone(),
            config.api_key.clone(),
            config.model.clone(),
        )
    }
}

impl OcrProvider for OpenAiCompatibleOcrProvider {
    fn recognize(&self, request: &OcrRequest) -> Result<OcrResult> {
        recognize_with_openai_compatible(
            &self.client,
            &self.base_url,
            &self.api_key,
            &self.model,
            request,
        )
    }
}

pub fn capture_interactive_screenshot() -> Result<Option<PathBuf>> {
    #[cfg(not(target_os = "macos"))]
    {
        bail!("interactive screenshot OCR is only implemented on macOS")
    }

    let output_path = next_screenshot_path();
    let output = Command::new("screencapture")
        .arg("-i")
        .arg("-x")
        .arg(&output_path)
        .output()
        .context("failed to launch macOS screencapture command")?;

    let screenshot_exists = output_path.is_file()
        && output_path
            .metadata()
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false);

    if output.status.success() {
        if screenshot_exists {
            return Ok(Some(output_path));
        }

        bail!("screencapture exited successfully but did not write an image");
    }

    if screenshot_exists {
        return Ok(Some(output_path));
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        return Ok(None);
    }

    Err(anyhow!("interactive screenshot failed: {stderr}"))
}

fn next_screenshot_path() -> PathBuf {
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);

    std::env::temp_dir().join(format!("wabity-ocr-{timestamp_ms}.png"))
}

fn aggregate_text(blocks: &[OcrTextBlock]) -> String {
    blocks
        .iter()
        .map(|block| block.text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn average_confidence(blocks: &[OcrTextBlock]) -> Option<f32> {
    let confidences = blocks
        .iter()
        .filter_map(|block| block.confidence)
        .collect::<Vec<_>>();

    if confidences.is_empty() {
        return None;
    }

    Some(confidences.iter().sum::<f32>() / confidences.len() as f32)
}

fn block_for_focus_point(blocks: &[OcrTextBlock], point: &OcrPoint) -> Option<OcrTextBlock> {
    blocks
        .iter()
        .min_by(|left, right| {
            let left_score = focus_match_score(&left.bounding_box, point);
            let right_score = focus_match_score(&right.bounding_box, point);
            left_score.total_cmp(&right_score)
        })
        .cloned()
}

fn focus_match_score(bounding_box: &OcrBoundingBox, point: &OcrPoint) -> f32 {
    let center_x = bounding_box.x + bounding_box.width / 2.0;
    let center_y = bounding_box.y + bounding_box.height / 2.0;
    let delta_x = center_x - point.x;
    let delta_y = center_y - point.y;
    let distance = delta_x * delta_x + delta_y * delta_y;

    if contains_point(bounding_box, point) {
        distance
    } else {
        distance + 10.0
    }
}

fn contains_point(bounding_box: &OcrBoundingBox, point: &OcrPoint) -> bool {
    let max_x = bounding_box.x + bounding_box.width;
    let max_y = bounding_box.y + bounding_box.height;

    point.x >= bounding_box.x && point.x <= max_x && point.y >= bounding_box.y && point.y <= max_y
}

fn recognize_with_openai_compatible(
    client: &reqwest::blocking::Client,
    base_url: &str,
    api_key: &str,
    model: &str,
    request: &OcrRequest,
) -> Result<OcrResult> {
    let image_path = Path::new(&request.image_path);
    if !image_path.is_file() {
        bail!(
            "ocr image path does not exist or is not a file: {}",
            image_path.display()
        );
    }

    let image_bytes = std::fs::read(image_path).with_context(|| {
        format!(
            "failed to read OCR image bytes from {}",
            image_path.display()
        )
    })?;
    let mime_type = infer_image_mime_type(image_path);
    let image_data_url = format!(
        "data:{mime_type};base64,{}",
        BASE64_STANDARD.encode(image_bytes)
    );

    let mut request_builder = client.post(format!("{base_url}/responses"));
    if !api_key.is_empty() {
        request_builder = request_builder.bearer_auth(api_key);
    }

    let response = request_builder
        .json(&json!({
            "model": model,
            "input": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "input_text",
                            "text": OPENAI_COMPATIBLE_OCR_PROMPT,
                        },
                        {
                            "type": "input_image",
                            "image_url": image_data_url,
                        }
                    ],
                }
            ],
        }))
        .send()
        .context("failed to send OCR request to OpenAI-compatible endpoint")?;
    let status = response.status();
    let body = response
        .text()
        .context("failed to read OpenAI-compatible OCR response body")?;

    if !status.is_success() {
        let message = extract_provider_error_message(&body);
        bail!("OpenAI-compatible OCR request failed with status {status}: {message}");
    }

    let parsed = parse_json_or_sse_payload(&body, "OpenAI-compatible OCR response body")
        .context("failed to parse OpenAI-compatible OCR response JSON")?;
    Ok(map_openai_compatible_response_to_result(parsed))
}

fn infer_image_mime_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
}

fn map_openai_compatible_response_to_result(response: Value) -> OcrResult {
    let text = extract_responses_text(&response).unwrap_or_default();

    OcrResult {
        text: text.trim().to_string(),
        language: None,
        confidence: None,
        blocks: Vec::new(),
        matched_block: None,
    }
}

#[cfg(target_os = "macos")]
fn recognize_with_macos_vision(request: &OcrRequest) -> Result<OcrResult> {
    use objc2::runtime::AnyObject;
    use objc2::AnyThread;
    use objc2_foundation::{NSArray, NSDictionary, NSString, NSURL};
    use objc2_vision::{
        VNImageOption, VNImageRequestHandler, VNRecognizeTextRequest, VNRequest,
        VNRequestTextRecognitionLevel,
    };

    let image_path = Path::new(&request.image_path);
    if !image_path.is_file() {
        bail!(
            "ocr image path does not exist or is not a file: {}",
            image_path.display()
        );
    }

    let path_string = image_path.to_string_lossy();
    let ns_path = NSString::from_str(&path_string);
    let image_url = NSURL::fileURLWithPath(&ns_path);
    let options: objc2::rc::Retained<NSDictionary<VNImageOption, AnyObject>> = NSDictionary::new();
    let handler = unsafe {
        VNImageRequestHandler::initWithURL_options(
            VNImageRequestHandler::alloc(),
            &image_url,
            &options,
        )
    };
    let request_object = unsafe { VNRecognizeTextRequest::init(VNRecognizeTextRequest::alloc()) };
    request_object.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    request_object.setUsesLanguageCorrection(true);
    request_object.setAutomaticallyDetectsLanguage(true);

    let requests: objc2::rc::Retained<NSArray<VNRequest>> =
        NSArray::from_slice(&[request_object.as_ref()]);
    handler
        .performRequests_error(&requests)
        .map_err(|error| anyhow!(error.to_string()))
        .with_context(|| {
            format!(
                "Vision failed to recognize text from {}",
                image_path.display()
            )
        })?;

    let blocks = request_object
        .results()
        .map(|observations| collect_text_blocks(&observations))
        .transpose()?
        .unwrap_or_default();

    let matched_block = request
        .focus_point
        .as_ref()
        .and_then(|point| block_for_focus_point(&blocks, point));
    let text = matched_block
        .as_ref()
        .map(|block| block.text.clone())
        .unwrap_or_else(|| aggregate_text(&blocks));
    let confidence = matched_block
        .as_ref()
        .and_then(|block| block.confidence)
        .or_else(|| average_confidence(&blocks));

    Ok(OcrResult {
        text,
        language: None,
        confidence,
        blocks,
        matched_block,
    })
}

#[cfg(target_os = "macos")]
fn collect_text_blocks(
    observations: &objc2_foundation::NSArray<objc2_vision::VNRecognizedTextObservation>,
) -> Result<Vec<OcrTextBlock>> {
    use objc2_core_graphics::{CGRectGetHeight, CGRectGetMinX, CGRectGetMinY, CGRectGetWidth};

    let mut blocks = Vec::new();

    for observation in observations.iter() {
        let candidates = observation.topCandidates(1);
        if candidates.is_empty() {
            continue;
        }

        let candidate = candidates.objectAtIndex(0);
        let text = candidate.string().to_string();
        if text.trim().is_empty() {
            continue;
        }

        let bounding_box = unsafe { observation.boundingBox() };
        blocks.push(OcrTextBlock {
            text,
            confidence: Some(candidate.confidence()),
            bounding_box: OcrBoundingBox {
                x: CGRectGetMinX(bounding_box) as f32,
                y: CGRectGetMinY(bounding_box) as f32,
                width: CGRectGetWidth(bounding_box) as f32,
                height: CGRectGetHeight(bounding_box) as f32,
            },
        });
    }

    Ok(blocks)
}

pub fn remove_screenshot_file(path: &Path) {
    if let Err(error) = std::fs::remove_file(path) {
        tracing::debug!(?error, path = %path.display(), "failed to delete temporary OCR screenshot");
    }
}

#[cfg(test)]
mod tests {
    use super::{
        aggregate_text, average_confidence, block_for_focus_point,
        map_openai_compatible_response_to_result, OcrBoundingBox, OcrPoint, OcrTextBlock,
    };
    use crate::infrastructure::openai_compatible::{
        extract_provider_error_message, normalize_base_url,
    };
    use serde_json::json;

    fn block(text: &str, x: f32, y: f32, width: f32, height: f32) -> OcrTextBlock {
        OcrTextBlock {
            text: text.to_string(),
            confidence: Some(0.9),
            bounding_box: OcrBoundingBox {
                x,
                y,
                width,
                height,
            },
        }
    }

    #[test]
    fn aggregate_text_keeps_line_order() {
        let blocks = vec![
            block("hello", 0.0, 0.0, 0.2, 0.2),
            block("world", 0.3, 0.3, 0.2, 0.2),
        ];
        assert_eq!(aggregate_text(&blocks), "hello\nworld");
    }

    #[test]
    fn focus_point_prefers_containing_block() {
        let blocks = vec![
            block("left", 0.1, 0.1, 0.2, 0.2),
            block("right", 0.7, 0.1, 0.2, 0.2),
        ];

        let matched = block_for_focus_point(&blocks, &OcrPoint { x: 0.78, y: 0.16 }).unwrap();

        assert_eq!(matched.text, "right");
    }

    #[test]
    fn average_confidence_ignores_missing_values() {
        let mut blocks = vec![
            block("left", 0.0, 0.0, 0.2, 0.2),
            block("right", 0.3, 0.3, 0.2, 0.2),
        ];
        blocks[1].confidence = None;

        assert_eq!(average_confidence(&blocks), Some(0.9));
    }

    #[test]
    fn normalize_base_url_trims_trailing_slash() {
        let normalized = normalize_base_url(
            " https://api.openai.example.com/v1/ ",
            "OpenAI-compatible base URL",
        )
        .unwrap();

        assert_eq!(normalized, "https://api.openai.example.com/v1");
    }

    #[test]
    fn map_openai_compatible_response_reads_text_content() {
        let response = json!({
            "output_text": "invoice total"
        });

        let result = map_openai_compatible_response_to_result(response);

        assert_eq!(result.text, "invoice total");
        assert!(result.blocks.is_empty());
        assert_eq!(result.confidence, None);
    }

    #[test]
    fn map_openai_compatible_response_reads_part_array() {
        let response = json!({
            "output": [
                {
                    "content": [
                        { "type": "output_text", "text": "line 1" },
                        { "type": "output_text", "text": "line 2" }
                    ]
                }
            ]
        });

        let result = map_openai_compatible_response_to_result(response);

        assert_eq!(result.text, "line 1\nline 2");
    }

    #[test]
    fn extract_openai_compatible_error_prefers_message_field() {
        let message = extract_provider_error_message(
            r#"{"error":{"message":"provider rejected image input"}}"#,
        );

        assert_eq!(message, "provider rejected image input");
    }
}
