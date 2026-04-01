use anyhow::{Context, Result};
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};

use crate::{
    domain::settings::LlmProviderConfig,
    infrastructure::openai_compatible::{normalize_base_url, OpenAiCompatibleClient},
};

use super::{
    config::infer_embedding_target_identity,
    model::{
        EmbeddingTargetIdentity, EMBEDDING_BATCH_CHAR_BUDGET, EMBEDDING_BATCH_COOLDOWN_ROUNDS,
        EMBEDDING_BATCH_GROWTH_DIVISOR, EMBEDDING_BATCH_GROWTH_SUCCESS_STREAK,
        EMBEDDING_BATCH_SIZE_DEFAULT, EMBEDDING_BATCH_SIZE_MAX, EMBEDDING_BATCH_SIZE_MIN,
        EMBEDDING_REQUEST_TIMEOUT,
    },
};

#[derive(Debug, Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingItem>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingItem {
    embedding: Vec<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct EmbeddingRequestStats {
    pub(super) largest_successful_batch_size: usize,
    pub(super) split_retry_count: usize,
}

impl EmbeddingRequestStats {
    fn record_success(&mut self, batch_size: usize) {
        self.largest_successful_batch_size = self.largest_successful_batch_size.max(batch_size);
    }

    fn record_split_retry(&mut self) {
        self.split_retry_count += 1;
    }

    pub(super) fn had_to_split(self) -> bool {
        self.split_retry_count > 0
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct EmbeddingBatchPlanner {
    pub(super) current_size: usize,
    pub(super) clean_success_streak: usize,
    pub(super) cooldown_rounds: usize,
}

impl Default for EmbeddingBatchPlanner {
    fn default() -> Self {
        Self {
            current_size: EMBEDDING_BATCH_SIZE_DEFAULT,
            clean_success_streak: 0,
            cooldown_rounds: 0,
        }
    }
}

impl EmbeddingBatchPlanner {
    pub(super) fn next_batch_end(self, inputs: &[String], start: usize) -> usize {
        let remaining = inputs.len().saturating_sub(start);
        if remaining == 0 {
            return start;
        }

        let item_limit = remaining.min(
            self.current_size
                .clamp(EMBEDDING_BATCH_SIZE_MIN, EMBEDDING_BATCH_SIZE_MAX),
        );
        let mut total_chars = 0usize;
        let mut end = start;

        while end < inputs.len() && end - start < item_limit {
            let input_chars = inputs[end].chars().count().max(1);
            if end > start && total_chars.saturating_add(input_chars) > EMBEDDING_BATCH_CHAR_BUDGET
            {
                break;
            }

            total_chars = total_chars.saturating_add(input_chars);
            end += 1;
            if total_chars >= EMBEDDING_BATCH_CHAR_BUDGET {
                break;
            }
        }

        end
    }

    pub(super) fn record_success(
        &mut self,
        requested_batch_size: usize,
        stats: EmbeddingRequestStats,
    ) {
        if stats.had_to_split() {
            self.current_size = stats
                .largest_successful_batch_size
                .max(EMBEDDING_BATCH_SIZE_MIN)
                .clamp(EMBEDDING_BATCH_SIZE_MIN, EMBEDDING_BATCH_SIZE_MAX);
            self.clean_success_streak = 0;
            self.cooldown_rounds = EMBEDDING_BATCH_COOLDOWN_ROUNDS;
            return;
        }

        if self.cooldown_rounds > 0 {
            self.cooldown_rounds -= 1;
            self.clean_success_streak = 0;
            return;
        }

        if requested_batch_size
            < self
                .current_size
                .clamp(EMBEDDING_BATCH_SIZE_MIN, EMBEDDING_BATCH_SIZE_MAX)
        {
            return;
        }

        self.clean_success_streak += 1;
        if self.clean_success_streak < EMBEDDING_BATCH_GROWTH_SUCCESS_STREAK {
            return;
        }

        self.clean_success_streak = 0;
        let growth = (self.current_size / EMBEDDING_BATCH_GROWTH_DIVISOR).max(1);
        self.current_size = self
            .current_size
            .saturating_add(growth)
            .clamp(EMBEDDING_BATCH_SIZE_MIN, EMBEDDING_BATCH_SIZE_MAX);
    }
}

pub(crate) fn build_embedding_client() -> Result<HttpClient> {
    HttpClient::builder()
        .timeout(EMBEDDING_REQUEST_TIMEOUT)
        .build()
        .context("failed to build embedding HTTP client")
}

pub(crate) fn text_fingerprint(text: &str) -> String {
    format!("{:x}", md5::compute(text.as_bytes()))
}

pub(crate) async fn request_embeddings(
    client: &HttpClient,
    provider: &LlmProviderConfig,
    inputs: &[String],
) -> Result<Vec<Vec<f32>>> {
    let (vectors, _) = request_embeddings_with_stats(client, provider, inputs).await?;
    Ok(vectors)
}

pub(super) async fn request_embeddings_with_stats(
    client: &HttpClient,
    provider: &LlmProviderConfig,
    inputs: &[String],
) -> Result<(Vec<Vec<f32>>, EmbeddingRequestStats)> {
    if inputs.is_empty() {
        return Ok((
            Vec::new(),
            EmbeddingRequestStats {
                largest_successful_batch_size: 0,
                split_retry_count: 0,
            },
        ));
    }

    let mut pending_batches = vec![(0usize, inputs.len())];
    let mut resolved_vectors = vec![None; inputs.len()];
    let mut stats = EmbeddingRequestStats {
        largest_successful_batch_size: 0,
        split_retry_count: 0,
    };
    while let Some((start, end)) = pending_batches.pop() {
        let batch_inputs = &inputs[start..end];
        match request_embeddings_batch(client, provider, batch_inputs).await {
            Ok(vectors) => {
                if vectors.len() != batch_inputs.len() {
                    anyhow::bail!(
                        "embedding provider returned {} vectors for {} inputs",
                        vectors.len(),
                        batch_inputs.len()
                    );
                }
                stats.record_success(batch_inputs.len());
                for (offset, vector) in vectors.into_iter().enumerate() {
                    resolved_vectors[start + offset] = Some(vector);
                }
            }
            Err(error) if is_embedding_batch_overloaded(&error) && batch_inputs.len() > 1 => {
                let midpoint = start + (batch_inputs.len() / 2);
                stats.record_split_retry();
                tracing::warn!(
                    batch_size = batch_inputs.len(),
                    retry_left = midpoint - start,
                    retry_right = end - midpoint,
                    "embedding batch overloaded; retrying with smaller batches"
                );
                pending_batches.push((midpoint, end));
                pending_batches.push((start, midpoint));
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to embed batch with {} input(s)", batch_inputs.len())
                });
            }
        }
    }

    let vectors = resolved_vectors
        .into_iter()
        .map(|vector| vector.context("embedding batch completed without a vector"))
        .collect::<Result<Vec<_>>>()?;
    Ok((vectors, stats))
}

async fn request_embeddings_batch(
    client: &HttpClient,
    provider: &LlmProviderConfig,
    inputs: &[String],
) -> Result<Vec<Vec<f32>>> {
    let client = OpenAiCompatibleClient::new_async(
        client,
        &provider.base_url,
        &provider.api_key,
        "embedding provider base URL",
    )?;
    let parsed: EmbeddingResponse = client
        .post_json(
            "/embeddings",
            &EmbeddingRequest {
                model: provider.model_name(),
                input: inputs,
            },
            "embeddings from provider",
            crate::infrastructure::openai_compatible::OpenAiCompatibleResponseFormat::Json,
        )
        .await?;
    Ok(parsed.data.into_iter().map(|item| item.embedding).collect())
}

fn is_embedding_timeout(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|source| source.downcast_ref::<reqwest::Error>())
        .any(reqwest::Error::is_timeout)
}

fn is_embedding_batch_overloaded(error: &anyhow::Error) -> bool {
    if is_embedding_timeout(error) {
        return true;
    }

    let message = error.to_string().to_ascii_lowercase();
    [
        "out of memory",
        "cuda out of memory",
        "resource exhausted",
        "payload too large",
        "request entity too large",
        "413 payload too large",
    ]
    .iter()
    .any(|pattern| message.contains(pattern))
}

pub(crate) fn embedding_fingerprint(provider: &LlmProviderConfig) -> Result<String> {
    let base_url = normalize_base_url(&provider.base_url, "embedding provider base URL")?;
    let target_identity = infer_embedding_target_identity(
        &base_url,
        provider.model_name(),
        provider.model_identity_hint.as_deref(),
    );
    let fingerprint_source = match target_identity {
        EmbeddingTargetIdentity::StableModel {
            namespace,
            model_identity,
        } => format!("v2\u{0}stable\u{0}{namespace}\u{0}{model_identity}"),
        EmbeddingTargetIdentity::EndpointBound {
            normalized_base_url,
            model_identity,
        } => format!("v2\u{0}endpoint\u{0}{normalized_base_url}\u{0}{model_identity}"),
    };
    Ok(format!("{:x}", md5::compute(fingerprint_source)))
}
