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
    let hits =
        search_similar_chunks(data_dir, trimmed_query, embedding_provider, candidate_limit).await?;
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
    requested_limit
        .saturating_mul(CANDIDATE_EXPANSION_FACTOR)
        .max(CANDIDATE_LIMIT_FLOOR)
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
        ranked_hit.hit.score >= effective_min_score
            && ranked_hit.combined_score >= relative_combined_floor
            && distance_ceiling.is_none_or(|ceiling| ranked_hit.hit.distance <= ceiling)
            && !is_low_information_hit(&ranked_hit.hit)
            && (ranked_hit.term_coverage >= term_coverage_floor
                || ranked_hit.exact_query_match
                || term_coverage_floor == 0.0)
            && (lexical_presence_floor == 0
                || ranked_hit.exact_query_match
                || ranked_hit.has_structural_anchor
                || ranked_hit.matched_query_terms >= lexical_presence_floor)
            && (matched_terms_floor == 0
                || ranked_hit.has_structural_anchor
                || ranked_hit.matched_query_terms >= matched_terms_floor)
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
    if !query_terms.is_empty() {
        let path_terms = query_terms
            .iter()
            .filter(|term| path.contains(term.as_str()))
            .count();
        let heading_terms = query_terms
            .iter()
            .filter(|term| headings.contains(term.as_str()))
            .count();
        score += path_terms as f32 / query_terms.len() as f32 * 0.35;
        score += heading_terms as f32 / query_terms.len() as f32 * 0.25;
        has_anchor = has_anchor || path_terms > 0 || heading_terms > 0;
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
    (best_hit.term_coverage >= 0.8 && best_hit.matched_query_terms >= 2)
        .then_some(best_hit.hit.distance + STRONG_QUERY_MAX_DISTANCE_DELTA)
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
    use super::{
        distance_to_score, expanded_candidate_limit, is_base64_like_chunk, is_heading_only_chunk,
        min_anchor_term_matches, min_lexical_presence, min_relevance_score, prune_search_hits,
        rerank_search_hits, RagSearchHit, RankedSearchHit,
    };

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
    fn search_hits_are_capped_per_file_before_final_limit() {
        let hits = (0..5)
            .map(|index| RankedSearchHit {
                combined_score: 1.4 - (index as f32 * 0.01),
                term_coverage: 1.0,
                exact_query_match: true,
                matched_query_terms: 2,
                has_structural_anchor: true,
                hit: RagSearchHit {
                    source_root: "/docs".to_string(),
                    absolute_path: "/docs/a.md".to_string(),
                    path: "~/docs/a.md".to_string(),
                    chunk_index: index,
                    line_start: index,
                    line_end: index + 1,
                    paragraph_line_start: index,
                    heading_path: Vec::new(),
                    text: format!("a-{index}"),
                    distance: 0.02 + (index as f32 * 0.01),
                    score: 0.82 - (index as f32 * 0.01),
                },
            })
            .chain((0..3).map(|index| RankedSearchHit {
                combined_score: 1.2 - (index as f32 * 0.01),
                term_coverage: 1.0,
                exact_query_match: true,
                matched_query_terms: 2,
                has_structural_anchor: true,
                hit: RagSearchHit {
                    source_root: "/docs".to_string(),
                    absolute_path: "/docs/b.md".to_string(),
                    path: "~/docs/b.md".to_string(),
                    chunk_index: index,
                    line_start: index,
                    line_end: index + 1,
                    paragraph_line_start: index,
                    heading_path: Vec::new(),
                    text: format!("b-{index}"),
                    distance: 0.05 + (index as f32 * 0.01),
                    score: 0.74 - (index as f32 * 0.01),
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
                    source_root: "/docs".to_string(),
                    absolute_path: "/docs/exact.md".to_string(),
                    path: "~/docs/exact.md".to_string(),
                    chunk_index: 0,
                    line_start: 1,
                    line_end: 2,
                    paragraph_line_start: 1,
                    heading_path: vec!["Alpha".to_string()],
                    text: "Alpha timeout root cause".to_string(),
                    distance: 0.08,
                    score: 0.92,
                },
            },
            RankedSearchHit {
                combined_score: 0.95,
                term_coverage: 0.5,
                exact_query_match: false,
                matched_query_terms: 1,
                has_structural_anchor: false,
                hit: RagSearchHit {
                    source_root: "/docs".to_string(),
                    absolute_path: "/docs/noise.md".to_string(),
                    path: "~/docs/noise.md".to_string(),
                    chunk_index: 0,
                    line_start: 1,
                    line_end: 2,
                    paragraph_line_start: 1,
                    heading_path: vec!["Noise".to_string()],
                    text: "Beta export notes".to_string(),
                    distance: 0.18,
                    score: 0.84,
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
                source_root: "/docs".to_string(),
                absolute_path: "/docs/a.md".to_string(),
                path: "~/docs/a.md".to_string(),
                chunk_index: 0,
                line_start: 1,
                line_end: 2,
                paragraph_line_start: 1,
                heading_path: vec!["Alpha".to_string()],
                text: "alpha timeout root cause".to_string(),
                distance: 0.2,
                score: 0.83,
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
                source_root: "/docs".to_string(),
                absolute_path: "/docs/a.md".to_string(),
                path: "~/docs/a.md".to_string(),
                chunk_index: 0,
                line_start: 1,
                line_end: 2,
                paragraph_line_start: 1,
                heading_path: vec!["国富论".to_string()],
                text: "国富论 亚当 斯密 经济学".to_string(),
                distance: 0.8,
                score: 0.55,
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
                    source_root: "/docs".to_string(),
                    absolute_path: "/docs/exact.md".to_string(),
                    path: "~/docs/exact.md".to_string(),
                    chunk_index: 0,
                    line_start: 1,
                    line_end: 2,
                    paragraph_line_start: 1,
                    heading_path: vec!["Alpha".to_string()],
                    text: "alpha timeout root cause".to_string(),
                    distance: 0.08,
                    score: 0.92,
                },
            },
            RankedSearchHit {
                combined_score: 1.2,
                term_coverage: 0.0,
                exact_query_match: false,
                matched_query_terms: 0,
                has_structural_anchor: false,
                hit: RagSearchHit {
                    source_root: "/docs".to_string(),
                    absolute_path: "/docs/semantic-noise.md".to_string(),
                    path: "~/docs/semantic-noise.md".to_string(),
                    chunk_index: 0,
                    line_start: 1,
                    line_end: 2,
                    paragraph_line_start: 1,
                    heading_path: vec!["General".to_string()],
                    text: "generic system checklist".to_string(),
                    distance: 0.1,
                    score: 0.9,
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
                    source_root: "/docs".to_string(),
                    absolute_path: "/docs/generic.md".to_string(),
                    path: "~/docs/generic.md".to_string(),
                    chunk_index: 0,
                    line_start: 1,
                    line_end: 2,
                    paragraph_line_start: 1,
                    heading_path: vec!["General".to_string()],
                    text: "General timeout checklist for all services.".to_string(),
                    distance: 0.05,
                    score: 0.95,
                },
                RagSearchHit {
                    source_root: "/docs".to_string(),
                    absolute_path: "/docs/exact.md".to_string(),
                    path: "~/docs/exact.md".to_string(),
                    chunk_index: 0,
                    line_start: 1,
                    line_end: 2,
                    paragraph_line_start: 1,
                    heading_path: vec!["Alpha".to_string(), "Root Cause".to_string()],
                    text: "Alpha timeout root cause analysis and mitigation.".to_string(),
                    distance: 0.08,
                    score: 0.92,
                },
            ],
        );

        assert_eq!(reranked[0].hit.absolute_path, "/docs/exact.md");
        assert!(reranked[0].combined_score > reranked[1].combined_score);
    }
}
