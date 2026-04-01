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
use crate::services::document_extract::DocumentKind;
use crate::services::rag::{self, RAG_TABLE_NAME};

const MAX_TOP_K: usize = 20;
const MAX_HITS_PER_FILE: usize = 2;
const CANDIDATE_EXPANSION_FACTOR: usize = 8;
const CANDIDATE_LIMIT_FLOOR: usize = 24;
const RELATIVE_RELEVANCE_RATIO: f32 = 0.7;
const HIGH_CONFIDENCE_MIN_SCORE: f32 = 0.6;
const MEDIUM_CONFIDENCE_MIN_SCORE: f32 = 0.5;
const LOW_CONFIDENCE_MIN_SCORE: f32 = 0.4;
const FALLBACK_MIN_SCORE: f32 = 0.3;
const STRONG_QUERY_MAX_DISTANCE_DELTA: f32 = 0.12;
const BASE64_LINE_MIN_LEN: usize = 24;
const HEADING_ONLY_MAX_VISIBLE_CHARS: usize = 24;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RagSearchHit {
    pub source_root: String,
    pub absolute_path: String,
    pub path: String,
    pub document_kind: DocumentKind,
    pub chunk_index: i32,
    pub line_start: Option<i32>,
    pub line_end: Option<i32>,
    pub paragraph_line_start: Option<i32>,
    pub page_start: Option<i32>,
    pub page_end: Option<i32>,
    pub heading_path: Vec<String>,
    pub anchor_label: Option<String>,
    pub text: String,
    pub distance: f32,
    pub score: f32,
    #[serde(skip_serializing)]
    pub(crate) vector_score: f32,
    #[serde(skip_serializing)]
    pub(crate) lexical_score: f32,
    #[serde(skip_serializing)]
    pub(crate) has_vector_signal: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RagSearchResult {
    pub query: String,
    pub hit_count: usize,
    pub pending_indexing: bool,
    pub hits: Vec<RagSearchHit>,
}

#[derive(Debug, Clone)]
struct RankedSearchHit {
    hit: RagSearchHit,
    combined_score: f32,
    term_coverage: f32,
    exact_query_match: bool,
    matched_query_terms: usize,
    has_structural_anchor: bool,
}

#[derive(Debug, Clone, Copy)]
struct LexicalMatchSignals {
    score: f32,
    term_coverage: f32,
    exact_query_match: bool,
    matched_query_terms: usize,
}

#[derive(Debug, Clone, Copy)]
struct StructuralMatchSignals {
    score: f32,
    has_anchor: bool,
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
    let (vector_hits, lexical_hits) = tokio::join!(
        search_similar_chunks(data_dir, trimmed_query, embedding_provider, candidate_limit),
        search_lexical_chunks(data_dir, trimmed_query, candidate_limit),
    );
    let hits = merge_search_hits(vector_hits?, lexical_hits?);
    let ranked_hits = rerank_search_hits(trimmed_query, hits);
    let filtered_hits = prune_search_hits(ranked_hits, top_k, min_score);
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
            "document_kind",
            "chunk_index",
            "line_start",
            "line_end",
            "paragraph_line_start",
            "page_start",
            "page_end",
            "heading_path",
            "anchor_label",
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
    let document_kinds = downcast_string_column(batch, "document_kind")?;
    let chunk_indexes = downcast_int32_column(batch, "chunk_index")?;
    let line_starts = downcast_int32_column(batch, "line_start")?;
    let line_ends = downcast_int32_column(batch, "line_end")?;
    let paragraph_line_starts = downcast_int32_column(batch, "paragraph_line_start")?;
    let page_starts = downcast_int32_column(batch, "page_start")?;
    let page_ends = downcast_int32_column(batch, "page_end")?;
    let heading_paths = downcast_string_column(batch, "heading_path")?;
    let anchor_labels = downcast_string_column(batch, "anchor_label")?;
    let texts = downcast_string_column(batch, "text")?;
    let distance_column = column_by_name(batch, "_distance")?;

    let mut hits = Vec::with_capacity(batch.num_rows());
    for row_index in 0..batch.num_rows() {
        let distance = float_value_at(distance_column.as_ref(), row_index)
            .with_context(|| format!("failed to read LanceDB distance at row {row_index}"))?;
        let vector_score = distance_to_score(distance);
        let absolute_path = absolute_paths.value(row_index).to_string();
        hits.push(RagSearchHit {
            source_root: source_roots.value(row_index).to_string(),
            path: rag::display_path_for_prompt(&absolute_path),
            absolute_path,
            document_kind: rag::parse_document_kind(document_kinds.value(row_index))?,
            chunk_index: chunk_indexes.value(row_index),
            line_start: optional_int32_value_at(line_starts, row_index),
            line_end: optional_int32_value_at(line_ends, row_index),
            paragraph_line_start: optional_int32_value_at(paragraph_line_starts, row_index),
            page_start: optional_int32_value_at(page_starts, row_index),
            page_end: optional_int32_value_at(page_ends, row_index),
            heading_path: rag::parse_heading_path(heading_paths.value(row_index))?,
            anchor_label: optional_string_value_at(anchor_labels, row_index),
            text: texts.value(row_index).to_string(),
            distance,
            score: vector_score,
            vector_score,
            lexical_score: 0.0,
            has_vector_signal: true,
        });
    }

    Ok(hits)
}

async fn search_lexical_chunks(
    data_dir: &Path,
    query: &str,
    top_k: usize,
) -> Result<Vec<RagSearchHit>> {
    let Some(match_query) = build_lexical_match_query(query) else {
        return Ok(Vec::new());
    };

    let metadata_path = rag::rag_metadata_database_path(data_dir);
    let rows = tokio::task::spawn_blocking(move || {
        rag::search_lexical_chunks(&metadata_path, &match_query, top_k)
    })
    .await
    .context("failed to join RAG lexical search task")??;

    Ok(rows
        .into_iter()
        .map(|row| {
            let lexical_score = bm25_rank_to_score(row.bm25_rank);
            RagSearchHit {
                source_root: row.source_root,
                absolute_path: row.absolute_path.clone(),
                path: rag::display_path_for_prompt(&row.absolute_path),
                document_kind: row.document_kind,
                chunk_index: row.chunk_index,
                line_start: row.line_start,
                line_end: row.line_end,
                paragraph_line_start: row.paragraph_line_start,
                page_start: row.page_start,
                page_end: row.page_end,
                heading_path: row.heading_path,
                anchor_label: row.anchor_label,
                text: row.text,
                distance: lexical_distance_for_score(lexical_score),
                score: lexical_score,
                vector_score: 0.0,
                lexical_score,
                has_vector_signal: false,
            }
        })
        .collect())
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

fn optional_int32_value_at(values: &Int32Array, row_index: usize) -> Option<i32> {
    (!values.is_null(row_index)).then(|| values.value(row_index))
}

fn optional_string_value_at(values: &StringArray, row_index: usize) -> Option<String> {
    (!values.is_null(row_index)).then(|| values.value(row_index).to_string())
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

fn bm25_rank_to_score(rank: f32) -> f32 {
    let magnitude = rank.abs();
    magnitude / (1.0 + magnitude)
}

fn lexical_distance_for_score(score: f32) -> f32 {
    (1.0 - score).clamp(0.0, 1.0)
}

fn expanded_candidate_limit(top_k: usize) -> usize {
    let requested_limit = top_k.clamp(1, MAX_TOP_K);
    requested_limit
        .saturating_mul(CANDIDATE_EXPANSION_FACTOR)
        .max(CANDIDATE_LIMIT_FLOOR)
}

fn merge_search_hits(
    vector_hits: Vec<RagSearchHit>,
    lexical_hits: Vec<RagSearchHit>,
) -> Vec<RagSearchHit> {
    let mut merged = HashMap::<String, RagSearchHit>::new();
    for hit in vector_hits.into_iter().chain(lexical_hits) {
        merged
            .entry(search_hit_key(&hit))
            .and_modify(|existing| merge_search_hit(existing, &hit))
            .or_insert(hit);
    }

    let mut hits = merged.into_values().collect::<Vec<_>>();
    hits.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                left.distance
                    .partial_cmp(&right.distance)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    hits
}

fn search_hit_key(hit: &RagSearchHit) -> String {
    format!("{}#{}", hit.absolute_path, hit.chunk_index)
}

fn merge_search_hit(existing: &mut RagSearchHit, incoming: &RagSearchHit) {
    existing.vector_score = existing.vector_score.max(incoming.vector_score);
    existing.lexical_score = existing.lexical_score.max(incoming.lexical_score);
    existing.has_vector_signal = existing.has_vector_signal || incoming.has_vector_signal;
    if incoming.has_vector_signal && incoming.distance < existing.distance {
        existing.distance = incoming.distance;
    }
    if existing.heading_path.is_empty() {
        existing.heading_path = incoming.heading_path.clone();
    }
    if existing.anchor_label.is_none() {
        existing.anchor_label = incoming.anchor_label.clone();
    }
    if existing.text.is_empty() {
        existing.text = incoming.text.clone();
    }
    existing.score = hybrid_candidate_score(existing.vector_score, existing.lexical_score);
    if !existing.has_vector_signal {
        existing.distance = lexical_distance_for_score(existing.lexical_score);
    }
}

fn hybrid_candidate_score(vector_score: f32, lexical_score: f32) -> f32 {
    let base = vector_score.max(lexical_score);
    if vector_score > 0.0 && lexical_score > 0.0 {
        (base + vector_score.min(lexical_score) * 0.15).min(1.0)
    } else {
        base
    }
}

fn prune_search_hits(
    hits: Vec<RankedSearchHit>,
    top_k: usize,
    min_score: f32,
) -> Vec<RagSearchHit> {
    let best_hit = match hits.first() {
        Some(hit) => hit,
        None => return Vec::new(),
    };
    let target_limit = top_k.clamp(1, MAX_TOP_K);
    let effective_min_score = min_relevance_score(best_hit.hit.score, min_score);
    let relative_combined_floor = best_hit.combined_score * RELATIVE_RELEVANCE_RATIO;
    let term_coverage_floor = min_term_coverage(best_hit.term_coverage);
    let matched_terms_floor = min_anchor_term_matches(best_hit);
    let lexical_presence_floor = min_lexical_presence(best_hit);
    let distance_ceiling = distance_ceiling_for_strong_query(best_hit);
    let mut file_buckets = HashMap::<String, Vec<RagSearchHit>>::new();
    let mut file_order = Vec::new();

    for ranked_hit in hits.into_iter().filter(|ranked_hit| {
        should_keep_ranked_hit(
            ranked_hit,
            effective_min_score,
            relative_combined_floor,
            term_coverage_floor,
            lexical_presence_floor,
            matched_terms_floor,
            distance_ceiling,
        )
    }) {
        let hit = ranked_hit.hit;
        if !file_buckets.contains_key(&hit.absolute_path) {
            file_order.push(hit.absolute_path.clone());
        }
        file_buckets
            .entry(hit.absolute_path.clone())
            .or_default()
            .push(hit);
    }

    let mut selected = Vec::new();
    let mut round = 0usize;
    while selected.len() < target_limit {
        let mut appended = false;
        for path in &file_order {
            let Some(bucket) = file_buckets.get(path) else {
                continue;
            };
            if round >= bucket.len() || round >= MAX_HITS_PER_FILE {
                continue;
            }

            selected.push(bucket[round].clone());
            appended = true;
            if selected.len() >= target_limit {
                break;
            }
        }

        if !appended {
            break;
        }
        round = round.saturating_add(1);
    }

    selected
}

fn should_keep_ranked_hit(
    ranked_hit: &RankedSearchHit,
    effective_min_score: f32,
    relative_combined_floor: f32,
    term_coverage_floor: f32,
    lexical_presence_floor: usize,
    matched_terms_floor: usize,
    distance_ceiling: Option<f32>,
) -> bool {
    ranked_hit.hit.score >= effective_min_score
        && ranked_hit.combined_score >= relative_combined_floor
        && within_distance_ceiling(ranked_hit, distance_ceiling)
        && !is_low_information_hit(&ranked_hit.hit)
        && meets_term_coverage_floor(ranked_hit, term_coverage_floor)
        && meets_lexical_presence_floor(ranked_hit, lexical_presence_floor)
        && meets_anchor_term_floor(ranked_hit, matched_terms_floor)
}

fn within_distance_ceiling(ranked_hit: &RankedSearchHit, distance_ceiling: Option<f32>) -> bool {
    distance_ceiling.is_none_or(|ceiling| {
        !ranked_hit.hit.has_vector_signal || ranked_hit.hit.distance <= ceiling
    })
}

fn meets_term_coverage_floor(ranked_hit: &RankedSearchHit, term_coverage_floor: f32) -> bool {
    ranked_hit.term_coverage >= term_coverage_floor
        || ranked_hit.exact_query_match
        || term_coverage_floor == 0.0
}

fn meets_lexical_presence_floor(
    ranked_hit: &RankedSearchHit,
    lexical_presence_floor: usize,
) -> bool {
    lexical_presence_floor == 0
        || ranked_hit.exact_query_match
        || ranked_hit.has_structural_anchor
        || ranked_hit.matched_query_terms >= lexical_presence_floor
}

fn meets_anchor_term_floor(ranked_hit: &RankedSearchHit, matched_terms_floor: usize) -> bool {
    matched_terms_floor == 0
        || ranked_hit.has_structural_anchor
        || ranked_hit.matched_query_terms >= matched_terms_floor
}

fn rerank_search_hits(query: &str, hits: Vec<RagSearchHit>) -> Vec<RankedSearchHit> {
    let normalized_query = normalize_text(query);
    let query_terms = extract_query_terms(query);
    let mut scored = hits
        .into_iter()
        .enumerate()
        .map(|(index, hit)| {
            let lexical_signals = lexical_match_signals(&hit, &normalized_query, &query_terms);
            let lexical = lexical_signals.score;
            let structural_signals =
                structural_match_signals(&hit, &normalized_query, &query_terms);
            let structural = structural_signals.score;
            let combined = hit.score + lexical * 0.45 + structural * 0.2;
            (
                combined,
                hit.distance,
                index,
                RankedSearchHit {
                    hit,
                    combined_score: combined,
                    term_coverage: lexical_signals.term_coverage,
                    exact_query_match: lexical_signals.exact_query_match,
                    matched_query_terms: lexical_signals.matched_query_terms,
                    has_structural_anchor: structural_signals.has_anchor,
                },
            )
        })
        .collect::<Vec<_>>();

    scored.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                left.1
                    .partial_cmp(&right.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| left.2.cmp(&right.2))
    });

    scored.into_iter().map(|(_, _, _, hit)| hit).collect()
}

fn lexical_match_signals(
    hit: &RagSearchHit,
    normalized_query: &str,
    query_terms: &[String],
) -> LexicalMatchSignals {
    let haystack = normalize_text(&format!(
        "{} {} {}",
        hit.path,
        hit.heading_path.join(" "),
        hit.text
    ));
    let mut score = 0.0;
    let exact_query_match = !normalized_query.is_empty() && haystack.contains(normalized_query);
    if exact_query_match {
        score += 1.0;
    }
    let mut term_coverage = 0.0;
    if !query_terms.is_empty() {
        let matched_terms = query_terms
            .iter()
            .filter(|term| haystack.contains(term.as_str()))
            .count();
        term_coverage = matched_terms as f32 / query_terms.len() as f32;
        score += term_coverage;
        return LexicalMatchSignals {
            score,
            term_coverage,
            exact_query_match,
            matched_query_terms: matched_terms,
        };
    }
    LexicalMatchSignals {
        score,
        term_coverage,
        exact_query_match,
        matched_query_terms: 0,
    }
}

fn structural_match_signals(
    hit: &RagSearchHit,
    normalized_query: &str,
    query_terms: &[String],
) -> StructuralMatchSignals {
    let path = normalize_text(&hit.path);
    let headings = normalize_text(&hit.heading_path.join(" "));
    let anchor = normalize_text(hit.anchor_label.as_deref().unwrap_or_default());
    let mut score = 0.0;
    let mut has_anchor = false;
    if !normalized_query.is_empty() && path.contains(normalized_query) {
        score += 0.8;
        has_anchor = true;
    }
    if !normalized_query.is_empty() && headings.contains(normalized_query) {
        score += 0.6;
        has_anchor = true;
    }
    if !normalized_query.is_empty() && !anchor.is_empty() && anchor.contains(normalized_query) {
        score += 0.3;
        has_anchor = true;
    }
    if !query_terms.is_empty() {
        let path_terms = query_terms
            .iter()
            .filter(|term| path.contains(term.as_str()))
            .count();
        let heading_terms = query_terms
            .iter()
            .filter(|term| headings.contains(term.as_str()))
            .count();
        let anchor_terms = query_terms
            .iter()
            .filter(|term| anchor.contains(term.as_str()))
            .count();
        score += path_terms as f32 / query_terms.len() as f32 * 0.35;
        score += heading_terms as f32 / query_terms.len() as f32 * 0.25;
        score += anchor_terms as f32 / query_terms.len() as f32 * 0.1;
        has_anchor = has_anchor || path_terms > 0 || heading_terms > 0 || anchor_terms > 0;
    }
    StructuralMatchSignals { score, has_anchor }
}

fn extract_query_terms(query: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for raw in query
        .split(|character: char| {
            !(character.is_alphanumeric() || character == '_' || character == '-')
        })
        .map(normalize_text)
        .filter(|term| term.chars().count() >= 2)
    {
        if !terms.contains(&raw) {
            terms.push(raw);
        }
    }

    if terms.is_empty() {
        let compact = normalize_text(query);
        if compact.chars().count() >= 2 {
            terms.push(compact);
        }
    }

    terms
}

fn normalize_text(text: &str) -> String {
    text.to_lowercase()
}

fn min_relevance_score(best_score: f32, requested_min_score: f32) -> f32 {
    let adaptive_floor = if best_score >= 0.8 {
        HIGH_CONFIDENCE_MIN_SCORE
    } else if best_score >= 0.65 {
        MEDIUM_CONFIDENCE_MIN_SCORE
    } else if best_score >= 0.5 {
        LOW_CONFIDENCE_MIN_SCORE
    } else {
        FALLBACK_MIN_SCORE
    };
    requested_min_score.max(adaptive_floor)
}

fn min_term_coverage(best_term_coverage: f32) -> f32 {
    if best_term_coverage >= 0.8 {
        0.75
    } else if best_term_coverage >= 0.6 {
        0.5
    } else {
        0.0
    }
}

fn min_lexical_presence(best_hit: &RankedSearchHit) -> usize {
    if best_hit.term_coverage >= 0.6 && best_hit.matched_query_terms >= 2 {
        1
    } else {
        0
    }
}

fn min_anchor_term_matches(best_hit: &RankedSearchHit) -> usize {
    if best_hit.term_coverage >= 0.8 && best_hit.matched_query_terms >= 2 {
        2
    } else {
        0
    }
}

fn distance_ceiling_for_strong_query(best_hit: &RankedSearchHit) -> Option<f32> {
    (best_hit.hit.has_vector_signal
        && best_hit.term_coverage >= 0.8
        && best_hit.matched_query_terms >= 2)
        .then_some(best_hit.hit.distance + STRONG_QUERY_MAX_DISTANCE_DELTA)
}

fn build_lexical_match_query(query: &str) -> Option<String> {
    let terms = extract_query_terms(query);
    if terms.is_empty() {
        return None;
    }

    Some(
        terms
            .into_iter()
            .map(|term| format!("\"{}\"", escape_fts_phrase(&term)))
            .collect::<Vec<_>>()
            .join(" OR "),
    )
}

fn escape_fts_phrase(term: &str) -> String {
    term.replace('"', "\"\"")
}

fn is_low_information_hit(hit: &RagSearchHit) -> bool {
    let trimmed = hit.text.trim();
    if trimmed.is_empty() {
        return true;
    }

    is_heading_only_chunk(trimmed) || is_base64_like_chunk(trimmed)
}

fn is_heading_only_chunk(text: &str) -> bool {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return true;
    }

    let visible = lines
        .iter()
        .map(|line| line.trim_start_matches('#').trim())
        .collect::<String>();
    lines.iter().all(|line| line.starts_with('#'))
        && visible.chars().count() <= HEADING_ONLY_MAX_VISIBLE_CHARS
}

fn is_base64_like_chunk(text: &str) -> bool {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() < 2 {
        return false;
    }

    lines.iter().all(|line| {
        line.len() >= BASE64_LINE_MIN_LEN
            && line.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '+' | '/' | '=')
            })
    })
}

#[cfg(test)]
mod tests {
    use crate::services::document_extract::DocumentKind;

    use super::{
        bm25_rank_to_score, build_lexical_match_query, distance_to_score, expanded_candidate_limit,
        hybrid_candidate_score, is_base64_like_chunk, is_heading_only_chunk,
        min_anchor_term_matches, min_lexical_presence, min_relevance_score, prune_search_hits,
        rerank_search_hits, RagSearchHit, RankedSearchHit,
    };

    fn test_search_hit(path: &str, text: impl Into<String>) -> RagSearchHit {
        RagSearchHit {
            source_root: "/docs".to_string(),
            absolute_path: path.to_string(),
            path: format!("~{}", path),
            document_kind: DocumentKind::Markdown,
            chunk_index: 0,
            line_start: Some(1),
            line_end: Some(2),
            paragraph_line_start: Some(1),
            page_start: None,
            page_end: None,
            heading_path: Vec::new(),
            anchor_label: None,
            text: text.into(),
            distance: 0.08,
            score: 0.92,
            vector_score: 0.92,
            lexical_score: 0.0,
            has_vector_signal: true,
        }
    }

    #[test]
    fn distance_score_is_clamped_to_positive_domain() {
        assert_eq!(distance_to_score(-2.0), 1.0);
        assert!(distance_to_score(0.5) < 1.0);
    }

    #[test]
    fn expanded_candidate_limit_allows_more_hits_than_requested_top_k() {
        assert_eq!(expanded_candidate_limit(1), 24);
        assert_eq!(expanded_candidate_limit(11), 88);
        assert_eq!(expanded_candidate_limit(20), 160);
    }

    #[test]
    fn bm25_rank_score_prefers_stronger_matches() {
        assert!(bm25_rank_to_score(-4.0) > bm25_rank_to_score(-0.5));
        assert!(bm25_rank_to_score(0.0) <= 0.01);
    }

    #[test]
    fn lexical_match_query_quotes_terms_for_fts() {
        assert_eq!(
            build_lexical_match_query("alpha timeout root cause").as_deref(),
            Some("\"alpha\" OR \"timeout\" OR \"root\" OR \"cause\"")
        );
    }

    #[test]
    fn hybrid_candidate_score_rewards_dual_signal_without_overflow() {
        assert!(hybrid_candidate_score(0.7, 0.6) > 0.7);
        assert!(hybrid_candidate_score(0.95, 0.9) <= 1.0);
    }

    #[test]
    fn search_hits_are_capped_per_file_before_final_limit() {
        let hits = (0..5)
            .map(|index| RankedSearchHit {
                combined_score: 1.4 - (index as f32 * 0.01),
                term_coverage: 1.0,
                exact_query_match: true,
                matched_query_terms: 2,
                has_structural_anchor: true,
                hit: RagSearchHit {
                    chunk_index: index,
                    line_start: Some(index),
                    line_end: Some(index + 1),
                    paragraph_line_start: Some(index),
                    distance: 0.02 + (index as f32 * 0.01),
                    score: 0.82 - (index as f32 * 0.01),
                    ..test_search_hit("/docs/a.md", format!("a-{index}"))
                },
            })
            .chain((0..3).map(|index| RankedSearchHit {
                combined_score: 1.2 - (index as f32 * 0.01),
                term_coverage: 1.0,
                exact_query_match: true,
                matched_query_terms: 2,
                has_structural_anchor: true,
                hit: RagSearchHit {
                    chunk_index: index,
                    line_start: Some(index),
                    line_end: Some(index + 1),
                    paragraph_line_start: Some(index),
                    distance: 0.05 + (index as f32 * 0.01),
                    score: 0.74 - (index as f32 * 0.01),
                    ..test_search_hit("/docs/b.md", format!("b-{index}"))
                },
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

    #[test]
    fn search_hits_drop_low_relevance_tail_even_without_explicit_min_score() {
        let hits = vec![
            RankedSearchHit {
                combined_score: 1.7,
                term_coverage: 1.0,
                exact_query_match: true,
                matched_query_terms: 3,
                has_structural_anchor: true,
                hit: RagSearchHit {
                    heading_path: vec!["Alpha".to_string()],
                    distance: 0.08,
                    score: 0.92,
                    ..test_search_hit("/docs/exact.md", "Alpha timeout root cause")
                },
            },
            RankedSearchHit {
                combined_score: 0.95,
                term_coverage: 0.5,
                exact_query_match: false,
                matched_query_terms: 1,
                has_structural_anchor: false,
                hit: RagSearchHit {
                    heading_path: vec!["Noise".to_string()],
                    distance: 0.18,
                    score: 0.84,
                    ..test_search_hit("/docs/noise.md", "Beta export notes")
                },
            },
        ];

        let pruned = prune_search_hits(hits, 4, 0.0);

        assert_eq!(pruned.len(), 1);
        assert_eq!(pruned[0].absolute_path, "/docs/exact.md");
    }

    #[test]
    fn min_relevance_score_raises_default_floor_for_strong_queries() {
        assert_eq!(min_relevance_score(0.85, 0.0), 0.6);
        assert_eq!(min_relevance_score(0.68, 0.0), 0.5);
        assert_eq!(min_relevance_score(0.52, 0.0), 0.4);
        assert_eq!(min_relevance_score(0.3, 0.0), 0.3);
        assert_eq!(min_relevance_score(0.85, 0.7), 0.7);
    }

    #[test]
    fn moderate_strong_query_requires_at_least_one_lexical_term() {
        let best_hit = RankedSearchHit {
            hit: RagSearchHit {
                heading_path: vec!["Alpha".to_string()],
                distance: 0.2,
                score: 0.83,
                ..test_search_hit("/docs/a.md", "alpha timeout root cause")
            },
            combined_score: 1.6,
            term_coverage: 0.66,
            exact_query_match: false,
            matched_query_terms: 2,
            has_structural_anchor: true,
        };

        assert_eq!(min_lexical_presence(&best_hit), 1);
    }

    #[test]
    fn strong_query_requires_at_least_two_anchor_terms() {
        let best_hit = RankedSearchHit {
            hit: RagSearchHit {
                heading_path: vec!["国富论".to_string()],
                distance: 0.8,
                score: 0.55,
                ..test_search_hit("/docs/a.md", "国富论 亚当 斯密 经济学")
            },
            combined_score: 1.4,
            term_coverage: 1.0,
            exact_query_match: true,
            matched_query_terms: 4,
            has_structural_anchor: true,
        };

        assert_eq!(min_anchor_term_matches(&best_hit), 2);
    }

    #[test]
    fn strong_query_drops_semantic_only_tail_without_lexical_presence() {
        let hits = vec![
            RankedSearchHit {
                combined_score: 1.5,
                term_coverage: 1.0,
                exact_query_match: true,
                matched_query_terms: 3,
                has_structural_anchor: true,
                hit: RagSearchHit {
                    heading_path: vec!["Alpha".to_string()],
                    distance: 0.08,
                    score: 0.92,
                    ..test_search_hit("/docs/exact.md", "alpha timeout root cause")
                },
            },
            RankedSearchHit {
                combined_score: 1.2,
                term_coverage: 0.0,
                exact_query_match: false,
                matched_query_terms: 0,
                has_structural_anchor: false,
                hit: RagSearchHit {
                    heading_path: vec!["General".to_string()],
                    distance: 0.1,
                    score: 0.9,
                    ..test_search_hit("/docs/semantic-noise.md", "generic system checklist")
                },
            },
        ];

        let pruned = prune_search_hits(hits, 4, 0.0);

        assert_eq!(pruned.len(), 1);
        assert_eq!(pruned[0].absolute_path, "/docs/exact.md");
    }

    #[test]
    fn low_information_detection_catches_heading_only_and_base64_chunks() {
        assert!(is_heading_only_chunk("# EC2\n"));
        assert!(is_heading_only_chunk("## 拓展\n"));
        assert!(is_base64_like_chunk(
            "nHYGNLkAvi8uwX4KzanQJyEk1FTSWDQQnmKj3f0V0goz\nGaGGh8g9TOI2+Uq+PcGLrjYszPVXquCmiOHDgikBwj+F\n"
        ));
        assert!(!is_base64_like_chunk("Alpha architecture fallback notes."));
    }

    #[test]
    fn rerank_prefers_exact_term_matches_over_generic_similarity_ties() {
        let reranked = rerank_search_hits(
            "alpha timeout root cause",
            vec![
                RagSearchHit {
                    heading_path: vec!["General".to_string()],
                    distance: 0.05,
                    score: 0.95,
                    ..test_search_hit(
                        "/docs/generic.md",
                        "General timeout checklist for all services.",
                    )
                },
                RagSearchHit {
                    heading_path: vec!["Alpha".to_string(), "Root Cause".to_string()],
                    distance: 0.08,
                    score: 0.92,
                    ..test_search_hit(
                        "/docs/exact.md",
                        "Alpha timeout root cause analysis and mitigation.",
                    )
                },
            ],
        );

        assert_eq!(reranked[0].hit.absolute_path, "/docs/exact.md");
        assert!(reranked[0].combined_score > reranked[1].combined_score);
    }
}
