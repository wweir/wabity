use std::{collections::HashMap, path::Path};

use anyhow::{bail, Context, Result};
use arrow_array::{Array, Float32Array, Float64Array, Int32Array, RecordBatch, StringArray};
use futures::TryStreamExt;
use lancedb::{
    connect,
    query::{ExecutableQuery, QueryBase, Select},
};
use serde::Serialize;

use crate::domain::settings::{LlmProviderConfig, LlmSettings, RagSettings};
use crate::services::rag::{self, RAG_TABLE_NAME};

const MAX_TOP_K: usize = 20;
const MAX_HITS_PER_FILE: usize = 2;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RagSearchHit {
    pub source_root: String,
    pub absolute_path: String,
    pub path: String,
    pub chunk_index: i32,
    pub line_start: i32,
    pub line_end: i32,
    pub paragraph_line_start: i32,
    pub heading_path: Vec<String>,
    pub text: String,
    pub distance: f32,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RagSearchResult {
    pub query: String,
    pub hit_count: usize,
    pub pending_indexing: bool,
    pub hits: Vec<RagSearchHit>,
}

pub async fn search_chunks(
    data_dir: &Path,
    query: &str,
    rag_settings: &RagSettings,
    llm_settings: &LlmSettings,
    top_k: usize,
    min_score: f32,
) -> Result<RagSearchResult> {
    let trimmed_query = query.trim();
    if trimmed_query.is_empty() {
        bail!("query 不能为空");
    }

    if rag_settings.source_directories.is_empty()
        || rag_settings
            .embedding_provider_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
    {
        bail!("RAG 尚未配置，请先在设置页配置文档目录并选择 embedding provider");
    }

    let embedding_provider = rag::resolve_embedding_provider(rag_settings, llm_settings)?;
    let candidate_limit = expanded_candidate_limit(top_k);
    let hits =
        search_similar_chunks(data_dir, trimmed_query, embedding_provider, candidate_limit).await?;
    let filtered_hits = prune_search_hits(hits, top_k, min_score);
    let metadata_path = rag::rag_metadata_database_path(data_dir);
    let pending_indexing = rag::metadata_store_has_pending_rows(&metadata_path)?;

    Ok(RagSearchResult {
        query: trimmed_query.to_string(),
        hit_count: filtered_hits.len(),
        pending_indexing,
        hits: filtered_hits,
    })
}

async fn search_similar_chunks(
    data_dir: &Path,
    query: &str,
    embedding_provider: &LlmProviderConfig,
    top_k: usize,
) -> Result<Vec<RagSearchHit>> {
    let database_path = rag::rag_database_path(data_dir);
    let db = connect(database_path.to_string_lossy().as_ref())
        .execute()
        .await
        .context("failed to open LanceDB database for RAG search")?;
    let table_names = db
        .table_names()
        .execute()
        .await
        .context("failed to list LanceDB tables for RAG search")?;
    if !table_names.iter().any(|name| name == RAG_TABLE_NAME) {
        return Ok(Vec::new());
    }

    let table = db
        .open_table(RAG_TABLE_NAME)
        .execute()
        .await
        .context("failed to open RAG chunk table")?;
    let client = rag::build_embedding_client()?;
    let vectors =
        rag::request_embeddings(&client, embedding_provider, &[query.to_string()]).await?;
    let query_vector = vectors
        .into_iter()
        .next()
        .context("embedding provider did not return query embedding")?;
    let limit = top_k.max(1);

    let stream = table
        .query()
        .only_if("chunk_state = 'active'")
        .select(Select::columns(&[
            "source_root",
            "absolute_path",
            "chunk_index",
            "line_start",
            "line_end",
            "paragraph_line_start",
            "heading_path",
            "text",
            "_distance",
        ]))
        .nearest_to(query_vector.as_slice())
        .context("failed to prepare RAG vector search query")?
        .limit(limit)
        .execute()
        .await
        .context("failed to execute RAG vector search")?;
    let batches = stream
        .try_collect::<Vec<_>>()
        .await
        .context("failed to collect RAG vector search batches")?;

    let mut hits = Vec::new();
    for batch in batches {
        hits.extend(parse_search_batch(&batch)?);
    }

    hits.sort_by(|left, right| {
        left.distance
            .partial_cmp(&right.distance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(hits)
}

fn parse_search_batch(batch: &RecordBatch) -> Result<Vec<RagSearchHit>> {
    if batch.num_rows() == 0 {
        return Ok(Vec::new());
    }

    let source_roots = downcast_string_column(batch, "source_root")?;
    let absolute_paths = downcast_string_column(batch, "absolute_path")?;
    let chunk_indexes = downcast_int32_column(batch, "chunk_index")?;
    let line_starts = downcast_int32_column(batch, "line_start")?;
    let line_ends = downcast_int32_column(batch, "line_end")?;
    let paragraph_line_starts = downcast_int32_column(batch, "paragraph_line_start")?;
    let heading_paths = downcast_string_column(batch, "heading_path")?;
    let texts = downcast_string_column(batch, "text")?;
    let distance_column = column_by_name(batch, "_distance")?;

    let mut hits = Vec::with_capacity(batch.num_rows());
    for row_index in 0..batch.num_rows() {
        let distance = float_value_at(distance_column.as_ref(), row_index)
            .with_context(|| format!("failed to read LanceDB distance at row {row_index}"))?;
        let absolute_path = absolute_paths.value(row_index).to_string();
        hits.push(RagSearchHit {
            source_root: source_roots.value(row_index).to_string(),
            path: rag::display_path_for_prompt(&absolute_path),
            absolute_path,
            chunk_index: chunk_indexes.value(row_index),
            line_start: line_starts.value(row_index),
            line_end: line_ends.value(row_index),
            paragraph_line_start: paragraph_line_starts.value(row_index),
            heading_path: rag::parse_heading_path(heading_paths.value(row_index))?,
            text: texts.value(row_index).to_string(),
            distance,
            score: distance_to_score(distance),
        });
    }

    Ok(hits)
}

fn downcast_string_column<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a StringArray> {
    column_by_name(batch, name)?
        .as_any()
        .downcast_ref::<StringArray>()
        .with_context(|| format!("column `{name}` is not a StringArray"))
}

fn downcast_int32_column<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a Int32Array> {
    column_by_name(batch, name)?
        .as_any()
        .downcast_ref::<Int32Array>()
        .with_context(|| format!("column `{name}` is not an Int32Array"))
}

fn column_by_name<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a std::sync::Arc<dyn Array>> {
    let index = batch
        .schema()
        .index_of(name)
        .with_context(|| format!("column `{name}` is missing from LanceDB result"))?;
    Ok(batch.column(index))
}

fn float_value_at(column: &dyn Array, row_index: usize) -> Result<f32> {
    if let Some(values) = column.as_any().downcast_ref::<Float32Array>() {
        return Ok(values.value(row_index));
    }
    if let Some(values) = column.as_any().downcast_ref::<Float64Array>() {
        return Ok(values.value(row_index) as f32);
    }

    bail!(
        "unsupported LanceDB distance column type: {:?}",
        column.data_type()
    )
}

fn distance_to_score(distance: f32) -> f32 {
    1.0 / (1.0 + distance.max(0.0))
}

fn expanded_candidate_limit(top_k: usize) -> usize {
    let requested_limit = top_k.clamp(1, MAX_TOP_K);
    requested_limit.saturating_mul(MAX_HITS_PER_FILE).max(1)
}

fn prune_search_hits(hits: Vec<RagSearchHit>, top_k: usize, min_score: f32) -> Vec<RagSearchHit> {
    let target_limit = top_k.clamp(1, MAX_TOP_K);
    let mut file_counts = HashMap::new();
    let mut selected = Vec::new();

    for hit in hits.into_iter().filter(|hit| hit.score >= min_score) {
        let file_count = file_counts
            .entry(hit.absolute_path.clone())
            .or_insert(0usize);
        if *file_count >= MAX_HITS_PER_FILE {
            continue;
        }

        *file_count += 1;
        selected.push(hit);
        if selected.len() >= target_limit {
            break;
        }
    }

    selected
}

#[cfg(test)]
mod tests {
    use super::{distance_to_score, expanded_candidate_limit, prune_search_hits, RagSearchHit};

    #[test]
    fn distance_score_is_clamped_to_positive_domain() {
        assert_eq!(distance_to_score(-2.0), 1.0);
        assert!(distance_to_score(0.5) < 1.0);
    }

    #[test]
    fn expanded_candidate_limit_allows_more_hits_than_requested_top_k() {
        assert_eq!(expanded_candidate_limit(1), 2);
        assert_eq!(expanded_candidate_limit(11), 22);
        assert_eq!(expanded_candidate_limit(20), 40);
    }

    #[test]
    fn search_hits_are_capped_per_file_before_final_limit() {
        let hits = (0..5)
            .map(|index| RagSearchHit {
                source_root: "/docs".to_string(),
                absolute_path: "/docs/a.md".to_string(),
                path: "~/docs/a.md".to_string(),
                chunk_index: index,
                line_start: index,
                line_end: index + 1,
                paragraph_line_start: index,
                heading_path: Vec::new(),
                text: format!("a-{index}"),
                distance: index as f32,
                score: 1.0 - (index as f32 * 0.01),
            })
            .chain((0..3).map(|index| RagSearchHit {
                source_root: "/docs".to_string(),
                absolute_path: "/docs/b.md".to_string(),
                path: "~/docs/b.md".to_string(),
                chunk_index: index,
                line_start: index,
                line_end: index + 1,
                paragraph_line_start: index,
                heading_path: Vec::new(),
                text: format!("b-{index}"),
                distance: (index + 10) as f32,
                score: 0.8 - (index as f32 * 0.01),
            }))
            .collect::<Vec<_>>();

        let pruned = prune_search_hits(hits, 4, 0.0);

        assert_eq!(pruned.len(), 4);
        assert_eq!(
            pruned
                .iter()
                .filter(|hit| hit.absolute_path == "/docs/a.md")
                .count(),
            2
        );
        assert_eq!(
            pruned
                .iter()
                .filter(|hit| hit.absolute_path == "/docs/b.md")
                .count(),
            2
        );
    }
}
