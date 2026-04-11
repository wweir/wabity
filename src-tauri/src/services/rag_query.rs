use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, OnceLock},
    time::Instant,
};

use anyhow::{bail, Context, Result};
use rusqlite::Connection;
use serde::Serialize;
use tokio::sync::Mutex as AsyncMutex;
use usearch::{ffi::Matches, Index};

use crate::domain::settings::{LlmProviderConfig, LlmSettings, RagSettings};
use crate::services::document_extract::DocumentKind;
use crate::services::rag::{self, open_vector_chunk_connection, vector_index_file_path};

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
const MAX_SEMANTIC_QUERIES: usize = 3;
const MAX_LEXICAL_QUERIES: usize = 4;
const VECTOR_SEMANTIC_CANDIDATE_FLOOR: usize = 12;
const LEXICAL_SEMANTIC_CANDIDATE_FLOOR: usize = 12;

const ENGLISH_QUERY_NOISE_TERMS: &[&str] = &[
    "a",
    "an",
    "the",
    "is",
    "are",
    "was",
    "were",
    "be",
    "been",
    "being",
    "do",
    "does",
    "did",
    "to",
    "for",
    "of",
    "in",
    "on",
    "at",
    "by",
    "with",
    "from",
    "about",
    "into",
    "show",
    "where",
    "which",
    "what",
    "when",
    "why",
    "how",
    "find",
    "locate",
    "look",
    "need",
    "want",
    "tell",
    "me",
    "documented",
    "explain",
];

const CHINESE_QUERY_NOISE_TERMS: &[&str] = &[
    "请",
    "帮我",
    "一下",
    "哪里",
    "在哪",
    "什么",
    "为什么",
    "怎么",
    "如何",
    "哪个",
    "解释",
    "给我",
    "找",
    "看看",
];

#[derive(Debug, Clone)]
struct SemanticQuery {
    text: String,
    weight: f32,
}

#[derive(Debug, Clone)]
struct LexicalQuery {
    match_query: String,
    weight: f32,
}

#[derive(Debug, Clone)]
struct QueryPlan {
    normalized_query: String,
    focus_query: String,
    normalized_focus_query: String,
    required_terms: Vec<String>,
    semantic_queries: Vec<SemanticQuery>,
    lexical_queries: Vec<LexicalQuery>,
}

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
    #[serde(skip_serializing)]
    pub(crate) retrieval_boost: f32,
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
    required_term_coverage: f32,
}

#[derive(Debug, Clone, Copy)]
struct StructuralMatchSignals {
    score: f32,
    has_anchor: bool,
}

#[derive(Debug, Clone, Copy)]
struct SearchRetentionPolicy {
    effective_min_score: f32,
    relative_combined_floor: f32,
    term_coverage_floor: f32,
    lexical_presence_floor: usize,
    matched_terms_floor: usize,
    distance_ceiling: Option<f32>,
}

pub async fn search_chunks(
    data_dir: &Path,
    query: &str,
    rag_settings: &RagSettings,
    llm_settings: &LlmSettings,
    top_k: usize,
    min_score: f32,
) -> Result<RagSearchResult> {
    let total_started_at = Instant::now();
    let search_span = tracing::info_span!(
        "rag_search_chunks",
        query_len = query.trim().chars().count(),
        top_k,
        min_score
    );
    let _search_span = search_span.enter();
    let trimmed_query = query.trim();
    if trimmed_query.is_empty() {
        bail!("query 不能为空");
    }
    let query_plan = build_query_plan(trimmed_query);
    tracing::debug!(
        focus_query = query_plan.focus_query,
        semantic_query_count = query_plan.semantic_queries.len(),
        lexical_query_count = query_plan.lexical_queries.len(),
        required_term_count = query_plan.required_terms.len(),
        "built RAG query plan"
    );

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
    let vector_limit = per_query_candidate_limit(
        candidate_limit,
        query_plan.semantic_queries.len(),
        VECTOR_SEMANTIC_CANDIDATE_FLOOR,
    );
    let lexical_limit = per_query_candidate_limit(
        candidate_limit,
        query_plan.lexical_queries.len(),
        LEXICAL_SEMANTIC_CANDIDATE_FLOOR,
    );
    let (vector_hits, lexical_hits) = tokio::join!(
        search_similar_chunks(data_dir, embedding_provider, &query_plan, vector_limit),
        search_lexical_chunks(data_dir, &query_plan, lexical_limit),
    );
    let hits = merge_search_hits(vector_hits?, lexical_hits?);
    let ranked_hits = rerank_search_hits(&query_plan, hits);
    let filtered_hits = prune_search_hits(ranked_hits, top_k, min_score);
    let sqlite_path = rag::rag_sqlite_database_path(data_dir);
    let pending_indexing = rag::rag_sqlite_has_pending_rows(&sqlite_path)?;
    tracing::info!(
        elapsed_ms = total_started_at.elapsed().as_millis(),
        hit_count = filtered_hits.len(),
        pending_indexing,
        "rag search completed"
    );

    Ok(RagSearchResult {
        query: trimmed_query.to_string(),
        hit_count: filtered_hits.len(),
        pending_indexing,
        hits: filtered_hits,
    })
}

#[derive(Debug, Clone)]
struct StoredSearchRow {
    source_root: String,
    absolute_path: String,
    document_kind: DocumentKind,
    chunk_index: i32,
    line_start: Option<i32>,
    line_end: Option<i32>,
    paragraph_line_start: Option<i32>,
    page_start: Option<i32>,
    page_end: Option<i32>,
    heading_path: Vec<String>,
    anchor_label: Option<String>,
    text: String,
}

fn rag_query_db_cache() -> &'static AsyncMutex<HashMap<String, Arc<Index>>> {
    static RAG_QUERY_DB_CACHE: OnceLock<AsyncMutex<HashMap<String, Arc<Index>>>> = OnceLock::new();

    RAG_QUERY_DB_CACHE.get_or_init(|| AsyncMutex::new(HashMap::new()))
}

fn rag_query_db_cache_key(database_path: &Path) -> String {
    database_path
        .canonicalize()
        .unwrap_or_else(|_| database_path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

pub(crate) async fn invalidate_rag_query_db_cache(database_path: &Path) {
    let normalized_path = rag_query_db_cache_key(database_path);
    let mut guard = rag_query_db_cache().lock().await;
    let removed = guard.remove(&normalized_path).is_some();
    tracing::debug!(removed, "invalidated cached USearch index for RAG query");
}

async fn get_rag_query_db(database_path: &Path) -> Result<Arc<Index>> {
    let normalized_path = rag_query_db_cache_key(database_path);
    let mut guard = rag_query_db_cache().lock().await;
    if let Some(index) = guard.get(&normalized_path) {
        return Ok(index.clone());
    }

    let started_at = Instant::now();
    let index = Arc::new(load_query_index(database_path)?);
    tracing::info!(
        elapsed_ms = started_at.elapsed().as_millis(),
        "opened cached USearch index for RAG query"
    );
    guard.insert(normalized_path, index.clone());
    Ok(index)
}

async fn search_similar_chunks(
    data_dir: &Path,
    embedding_provider: &LlmProviderConfig,
    query_plan: &QueryPlan,
    top_k: usize,
) -> Result<Vec<RagSearchHit>> {
    let vector_search_started_at = Instant::now();
    let database_path = rag::rag_database_path(data_dir);
    let index_path = vector_index_file_path(&database_path);
    if !index_path.exists() {
        return Ok(Vec::new());
    }

    let index = get_rag_query_db(&database_path).await?;
    let client = rag::build_embedding_client()?;
    let semantic_queries = query_plan
        .semantic_queries
        .iter()
        .map(|query| query.text.clone())
        .collect::<Vec<_>>();
    let vectors = rag::request_embeddings(&client, embedding_provider, &semantic_queries).await?;
    let limit = top_k.max(1);
    let query_weights = query_plan
        .semantic_queries
        .iter()
        .map(|query| query.weight)
        .collect::<Vec<_>>();
    let mut hits = tokio::task::spawn_blocking(move || {
        let connection = open_vector_chunk_connection(&database_path)?;
        let mut hits = Vec::new();

        for (query_index, query_vector) in vectors.into_iter().enumerate() {
            let query_weight = query_weights.get(query_index).copied().unwrap_or(0.0);
            let matches = index
                .search(query_vector.as_slice(), limit)
                .context("failed to execute USearch vector query")?;
            let mut batch_hits = load_search_hits(&connection, &matches)?;
            for hit in &mut batch_hits {
                hit.retrieval_boost = hit.retrieval_boost.max(query_weight);
            }
            hits.extend(batch_hits);
        }

        Ok::<_, anyhow::Error>(hits)
    })
    .await
    .context("failed to join RAG vector search task")??;

    hits.sort_by(|left, right| {
        left.distance
            .partial_cmp(&right.distance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    tracing::info!(
        elapsed_ms = vector_search_started_at.elapsed().as_millis(),
        hit_count = hits.len(),
        limit,
        "rag vector search completed"
    );
    Ok(hits)
}

async fn search_lexical_chunks(
    data_dir: &Path,
    query_plan: &QueryPlan,
    top_k: usize,
) -> Result<Vec<RagSearchHit>> {
    if query_plan.lexical_queries.is_empty() {
        return Ok(Vec::new());
    }

    let sqlite_path = rag::rag_sqlite_database_path(data_dir);
    let lexical_queries = query_plan.lexical_queries.clone();
    let rows = tokio::task::spawn_blocking(move || {
        let mut merged = HashMap::new();
        for query in lexical_queries {
            for row in rag::search_lexical_chunks(&sqlite_path, &query.match_query, top_k)? {
                let score = bm25_rank_to_score(row.bm25_rank) + query.weight;
                let key = format!("{}#{}", row.absolute_path, row.chunk_index);
                merged
                    .entry(key)
                    .and_modify(|existing: &mut (_, f32)| {
                        if score > existing.1 {
                            *existing = (row.clone(), score);
                        }
                    })
                    .or_insert((row, score));
            }
        }
        Ok::<_, anyhow::Error>(merged.into_values().collect::<Vec<_>>())
    })
    .await
    .context("failed to join RAG lexical search task")??;

    Ok(rows
        .into_iter()
        .map(|(row, lexical_score)| RagSearchHit {
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
            retrieval_boost: lexical_score.min(0.2),
        })
        .collect())
}

fn load_query_index(database_path: &Path) -> Result<Index> {
    let dimensions = rag::load_active_vector_dimensions(database_path)?.unwrap_or(1);
    let options = rag::build_usearch_index_options(dimensions);
    let index = Index::new(&options).context("failed to create USearch query index")?;

    let index_path = vector_index_file_path(database_path);
    if index_path.exists() {
        index
            .load(index_path.to_string_lossy().as_ref())
            .with_context(|| format!("failed to load USearch index: {}", index_path.display()))?;
    }
    Ok(index)
}

fn load_search_hits(connection: &Connection, matches: &Matches) -> Result<Vec<RagSearchHit>> {
    if matches.keys.is_empty() {
        return Ok(Vec::new());
    }

    let rows_by_key = load_search_rows_by_keys(connection, &matches.keys)?;
    let mut hits = Vec::with_capacity(matches.keys.len());
    for (vector_key, distance) in matches.keys.iter().zip(matches.distances.iter()) {
        let Some(row) = rows_by_key.get(vector_key) else {
            continue;
        };
        let vector_score = distance_to_score(*distance);
        hits.push(RagSearchHit {
            source_root: row.source_root.clone(),
            path: rag::display_path_for_prompt(&row.absolute_path),
            absolute_path: row.absolute_path.clone(),
            document_kind: row.document_kind,
            chunk_index: row.chunk_index,
            line_start: row.line_start,
            line_end: row.line_end,
            paragraph_line_start: row.paragraph_line_start,
            page_start: row.page_start,
            page_end: row.page_end,
            heading_path: row.heading_path.clone(),
            anchor_label: row.anchor_label.clone(),
            text: row.text.clone(),
            distance: *distance,
            score: vector_score,
            vector_score,
            lexical_score: 0.0,
            has_vector_signal: true,
            retrieval_boost: 0.0,
        });
    }
    Ok(hits)
}

fn load_search_rows_by_keys(
    connection: &Connection,
    keys: &[u64],
) -> Result<HashMap<u64, StoredSearchRow>> {
    let placeholders = (0..keys.len())
        .map(|index| format!("?{}", index + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "
        SELECT
            vector_key,
            source_root,
            absolute_path,
            document_kind,
            chunk_index,
            line_start,
            line_end,
            paragraph_line_start,
            page_start,
            page_end,
            heading_path_json,
            anchor_label,
            text
        FROM rag_chunks
        WHERE chunk_state = 'active'
          AND vector_key IN ({placeholders})
        "
    );
    let mut statement = connection
        .prepare(&sql)
        .context("failed to prepare rag chunk search row query")?;
    let numeric_keys = keys
        .iter()
        .map(|key| i64::try_from(*key).context("vector key does not fit into SQLite INTEGER"))
        .collect::<Result<Vec<_>>>()?;
    let params = rusqlite::params_from_iter(numeric_keys.iter());
    let mut rows = statement
        .query(params)
        .context("failed to execute rag chunk search row query")?;
    let mut results = HashMap::new();
    while let Some(row) = rows
        .next()
        .context("failed to step rag chunk search rows")?
    {
        let vector_key_i64: i64 = row.get(0)?;
        let vector_key = u64::try_from(vector_key_i64).context("vector_key is negative")?;
        let document_kind_raw: String = row.get(3)?;
        let heading_path_raw: String = row.get(10)?;
        results.insert(
            vector_key,
            StoredSearchRow {
                source_root: row.get(1)?,
                absolute_path: row.get(2)?,
                document_kind: rag::parse_document_kind(&document_kind_raw)?,
                chunk_index: row.get(4)?,
                line_start: row.get(5)?,
                line_end: row.get(6)?,
                paragraph_line_start: row.get(7)?,
                page_start: row.get(8)?,
                page_end: row.get(9)?,
                heading_path: rag::parse_heading_path(&heading_path_raw)?,
                anchor_label: row.get(11)?,
                text: row.get(12)?,
            },
        );
    }
    Ok(results)
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
    existing.retrieval_boost = existing.retrieval_boost.max(incoming.retrieval_boost);
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
    let retention_policy = SearchRetentionPolicy {
        effective_min_score: min_relevance_score(best_hit.hit.score, min_score),
        relative_combined_floor: best_hit.combined_score * RELATIVE_RELEVANCE_RATIO,
        term_coverage_floor: min_term_coverage(best_hit.term_coverage),
        lexical_presence_floor: min_lexical_presence(best_hit),
        matched_terms_floor: min_anchor_term_matches(best_hit),
        distance_ceiling: distance_ceiling_for_strong_query(best_hit),
    };
    let mut file_buckets = HashMap::<String, Vec<RagSearchHit>>::new();
    let mut file_order = Vec::new();

    for ranked_hit in hits
        .into_iter()
        .filter(|ranked_hit| should_keep_ranked_hit(ranked_hit, retention_policy))
    {
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
    retention_policy: SearchRetentionPolicy,
) -> bool {
    ranked_hit.hit.score >= retention_policy.effective_min_score
        && ranked_hit.combined_score >= retention_policy.relative_combined_floor
        && within_distance_ceiling(ranked_hit, retention_policy.distance_ceiling)
        && !is_low_information_hit(&ranked_hit.hit)
        && meets_term_coverage_floor(ranked_hit, retention_policy.term_coverage_floor)
        && meets_lexical_presence_floor(ranked_hit, retention_policy.lexical_presence_floor)
        && meets_anchor_term_floor(ranked_hit, retention_policy.matched_terms_floor)
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

fn rerank_search_hits(query_plan: &QueryPlan, hits: Vec<RagSearchHit>) -> Vec<RankedSearchHit> {
    let mut scored = hits
        .into_iter()
        .enumerate()
        .map(|(index, hit)| {
            let lexical_signals = lexical_match_signals(&hit, query_plan);
            let lexical = lexical_signals.score;
            let structural_signals = structural_match_signals(&hit, query_plan);
            let structural = structural_signals.score;
            let combined = hit.score
                + lexical * 0.45
                + structural * 0.2
                + hit.retrieval_boost * 0.25
                + lexical_signals.required_term_coverage * 0.15;
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

fn lexical_match_signals(hit: &RagSearchHit, query_plan: &QueryPlan) -> LexicalMatchSignals {
    let haystack = normalize_text(&format!(
        "{} {} {}",
        hit.path,
        hit.heading_path.join(" "),
        hit.text
    ));
    let mut score = 0.0;
    let exact_query_match = !query_plan.normalized_focus_query.is_empty()
        && haystack.contains(query_plan.normalized_focus_query.as_str());
    if exact_query_match {
        score += 1.0;
    }
    if !query_plan.normalized_query.is_empty()
        && query_plan.normalized_query != query_plan.normalized_focus_query
        && haystack.contains(query_plan.normalized_query.as_str())
    {
        score += 0.3;
    }
    let mut term_coverage = 0.0;
    let mut required_term_coverage = 0.0;
    if !query_plan.required_terms.is_empty() {
        let matched_terms = query_plan
            .required_terms
            .iter()
            .filter(|term| haystack.contains(term.as_str()))
            .count();
        term_coverage = matched_terms as f32 / query_plan.required_terms.len() as f32;
        required_term_coverage = term_coverage;
        score += term_coverage * 1.2;
        return LexicalMatchSignals {
            score,
            term_coverage,
            exact_query_match,
            matched_query_terms: matched_terms,
            required_term_coverage,
        };
    }
    LexicalMatchSignals {
        score,
        term_coverage,
        exact_query_match,
        matched_query_terms: 0,
        required_term_coverage,
    }
}

fn structural_match_signals(hit: &RagSearchHit, query_plan: &QueryPlan) -> StructuralMatchSignals {
    let path = normalize_text(&hit.path);
    let headings = normalize_text(&hit.heading_path.join(" "));
    let anchor = normalize_text(hit.anchor_label.as_deref().unwrap_or_default());
    let mut score = 0.0;
    let mut has_anchor = false;
    if !query_plan.normalized_focus_query.is_empty()
        && path.contains(query_plan.normalized_focus_query.as_str())
    {
        score += 0.8;
        has_anchor = true;
    }
    if !query_plan.normalized_focus_query.is_empty()
        && headings.contains(query_plan.normalized_focus_query.as_str())
    {
        score += 0.6;
        has_anchor = true;
    }
    if !query_plan.normalized_focus_query.is_empty()
        && !anchor.is_empty()
        && anchor.contains(query_plan.normalized_focus_query.as_str())
    {
        score += 0.3;
        has_anchor = true;
    }
    if !query_plan.required_terms.is_empty() {
        let path_terms = query_plan
            .required_terms
            .iter()
            .filter(|term| path.contains(term.as_str()))
            .count();
        let heading_terms = query_plan
            .required_terms
            .iter()
            .filter(|term| headings.contains(term.as_str()))
            .count();
        let anchor_terms = query_plan
            .required_terms
            .iter()
            .filter(|term| anchor.contains(term.as_str()))
            .count();
        score += path_terms as f32 / query_plan.required_terms.len() as f32 * 0.35;
        score += heading_terms as f32 / query_plan.required_terms.len() as f32 * 0.25;
        score += anchor_terms as f32 / query_plan.required_terms.len() as f32 * 0.1;
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

fn build_query_plan(query: &str) -> QueryPlan {
    let raw_query = query.trim().to_string();
    let normalized_query = normalize_text(&raw_query);
    let raw_terms = extract_query_terms(&raw_query);
    let required_terms = filter_query_noise_terms(&raw_terms);
    let focus_terms = preferred_query_terms(&required_terms, &raw_terms);
    let focus_query = if focus_terms.is_empty() {
        raw_query.clone()
    } else {
        focus_terms.join(" ")
    };
    let normalized_focus_query = normalize_text(&focus_query);
    let mut semantic_queries = vec![SemanticQuery {
        text: raw_query.clone(),
        weight: 0.0,
    }];
    push_semantic_query(
        &mut semantic_queries,
        focus_query.clone(),
        0.12,
        MAX_SEMANTIC_QUERIES,
    );
    if let Some(path_focused_query) = path_focused_query(&raw_query) {
        push_semantic_query(
            &mut semantic_queries,
            path_focused_query,
            0.08,
            MAX_SEMANTIC_QUERIES,
        );
    }

    let mut lexical_queries = Vec::new();
    if !normalized_focus_query.is_empty() && normalized_focus_query != normalized_query {
        push_lexical_query(
            &mut lexical_queries,
            format!("\"{}\"", escape_fts_phrase(&normalized_focus_query)),
            0.25,
            MAX_LEXICAL_QUERIES,
        );
    }
    if let Some(and_query) = build_term_conjunction_query(&required_terms) {
        push_lexical_query(&mut lexical_queries, and_query, 0.15, MAX_LEXICAL_QUERIES);
    }
    if let Some(or_query) =
        build_term_disjunction_query(preferred_query_terms(&required_terms, &raw_terms))
    {
        push_lexical_query(&mut lexical_queries, or_query, 0.0, MAX_LEXICAL_QUERIES);
    }
    if let Some(path_query) = build_term_disjunction_query(&path_focused_terms(&raw_query)) {
        push_lexical_query(&mut lexical_queries, path_query, 0.1, MAX_LEXICAL_QUERIES);
    }

    QueryPlan {
        normalized_query,
        focus_query,
        normalized_focus_query,
        required_terms,
        semantic_queries,
        lexical_queries,
    }
}

fn preferred_query_terms<'a>(
    required_terms: &'a [String],
    raw_terms: &'a [String],
) -> &'a [String] {
    if required_terms.is_empty() {
        raw_terms
    } else {
        required_terms
    }
}

fn push_semantic_query(queries: &mut Vec<SemanticQuery>, text: String, weight: f32, limit: usize) {
    let text = text.trim();
    if text.is_empty()
        || queries
            .iter()
            .any(|query| query.text.eq_ignore_ascii_case(text))
        || queries.len() >= limit
    {
        return;
    }
    queries.push(SemanticQuery {
        text: text.to_string(),
        weight,
    });
}

fn push_lexical_query(
    queries: &mut Vec<LexicalQuery>,
    match_query: String,
    weight: f32,
    limit: usize,
) {
    let match_query = match_query.trim();
    if match_query.is_empty()
        || queries
            .iter()
            .any(|query| query.match_query.eq_ignore_ascii_case(match_query))
        || queries.len() >= limit
    {
        return;
    }
    queries.push(LexicalQuery {
        match_query: match_query.to_string(),
        weight,
    });
}

fn filter_query_noise_terms(terms: &[String]) -> Vec<String> {
    terms
        .iter()
        .filter(|term| is_informative_query_term(term))
        .cloned()
        .collect()
}

fn is_informative_query_term(term: &str) -> bool {
    let normalized = normalize_text(term);
    if normalized.is_empty() {
        return false;
    }
    !ENGLISH_QUERY_NOISE_TERMS.contains(&normalized.as_str())
        && !CHINESE_QUERY_NOISE_TERMS.contains(&normalized.as_str())
}

fn path_focused_query(query: &str) -> Option<String> {
    let terms = path_focused_terms(query);
    (!terms.is_empty()).then(|| terms.join(" "))
}

fn path_focused_terms(query: &str) -> Vec<String> {
    query
        .split(|character: char| {
            !(character.is_alphanumeric() || matches!(character, '_' | '-' | '/' | '.' | ':'))
        })
        .flat_map(|segment| segment.split(['/', '.', ':']))
        .map(normalize_text)
        .filter(|term| term.chars().count() >= 2 && is_informative_query_term(term))
        .fold(Vec::new(), |mut acc, term| {
            if !acc.contains(&term) {
                acc.push(term);
            }
            acc
        })
}

fn build_term_conjunction_query(terms: &[String]) -> Option<String> {
    if terms.len() < 2 {
        return None;
    }
    Some(
        terms
            .iter()
            .map(|term| format!("\"{}\"", escape_fts_phrase(term)))
            .collect::<Vec<_>>()
            .join(" AND "),
    )
}

fn build_term_disjunction_query(terms: &[String]) -> Option<String> {
    if terms.is_empty() {
        return None;
    }
    Some(
        terms
            .iter()
            .map(|term| format!("\"{}\"", escape_fts_phrase(term)))
            .collect::<Vec<_>>()
            .join(" OR "),
    )
}

fn per_query_candidate_limit(total_limit: usize, query_count: usize, floor: usize) -> usize {
    let divisor = query_count.max(1);
    total_limit.saturating_div(divisor).max(floor)
}

#[cfg(test)]
fn build_lexical_match_query(query: &str) -> Option<String> {
    let plan = build_query_plan(query);
    plan.lexical_queries
        .into_iter()
        .next()
        .map(|query| query.match_query)
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
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::services::document_extract::DocumentKind;

    use super::{
        bm25_rank_to_score, build_lexical_match_query, build_query_plan, distance_to_score,
        expanded_candidate_limit, get_rag_query_db, hybrid_candidate_score,
        invalidate_rag_query_db_cache, is_base64_like_chunk, is_heading_only_chunk,
        min_anchor_term_matches, min_lexical_presence, min_relevance_score, prune_search_hits,
        rag_query_db_cache, rag_query_db_cache_key, rerank_search_hits, RagSearchHit,
        RankedSearchHit,
    };

    static NEXT_RAG_QUERY_CACHE_TEST_ID: AtomicU64 = AtomicU64::new(0);

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
            retrieval_boost: 0.0,
        }
    }

    fn next_temp_rag_query_db_path() -> PathBuf {
        let suffix = NEXT_RAG_QUERY_CACHE_TEST_ID.fetch_add(1, Ordering::Relaxed);
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default();
        std::env::temp_dir().join(format!("wabity-rag-query-cache-{timestamp_ms}-{suffix}"))
    }

    #[tokio::test]
    async fn invalidating_cached_rag_query_db_removes_cached_connection() {
        let database_path = next_temp_rag_query_db_path();
        tokio::fs::create_dir_all(&database_path)
            .await
            .expect("create temporary RAG query directory");

        get_rag_query_db(&database_path)
            .await
            .expect("open cached USearch index");

        let normalized_path = rag_query_db_cache_key(&database_path);
        assert!(rag_query_db_cache()
            .lock()
            .await
            .contains_key(&normalized_path));

        invalidate_rag_query_db_cache(&database_path).await;

        assert!(!rag_query_db_cache()
            .lock()
            .await
            .contains_key(&normalized_path));

        let _ = tokio::fs::remove_dir_all(&database_path).await;
    }

    #[tokio::test]
    async fn rag_query_db_cache_reuses_equivalent_database_paths() {
        let database_path = next_temp_rag_query_db_path();
        tokio::fs::create_dir_all(&database_path)
            .await
            .expect("create temporary RAG query directory");
        let canonical_path = database_path
            .canonicalize()
            .expect("canonicalize temporary RAG query directory");
        let dotted_path = canonical_path.join(".");
        let normalized_path = rag_query_db_cache_key(&canonical_path);
        invalidate_rag_query_db_cache(&canonical_path).await;
        invalidate_rag_query_db_cache(&dotted_path).await;

        let first = get_rag_query_db(&canonical_path)
            .await
            .expect("open cached USearch index for canonical path");
        let second = get_rag_query_db(&dotted_path)
            .await
            .expect("reuse cached USearch index for equivalent path");

        assert!(Arc::ptr_eq(&first, &second));
        assert!(rag_query_db_cache()
            .lock()
            .await
            .contains_key(&normalized_path));

        invalidate_rag_query_db_cache(&dotted_path).await;
        assert!(!rag_query_db_cache()
            .lock()
            .await
            .contains_key(&normalized_path));

        let _ = tokio::fs::remove_dir_all(&database_path).await;
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
            Some("\"alpha\" AND \"timeout\" AND \"root\" AND \"cause\"")
        );
    }

    #[test]
    fn query_plan_rewrites_question_wrapper_into_focus_terms() {
        let plan = build_query_plan("where is the alpha timeout root cause documented?");

        assert_eq!(plan.focus_query, "alpha timeout root cause");
        assert!(plan
            .semantic_queries
            .iter()
            .any(|query| query.text == "alpha timeout root cause"));
        assert!(plan.lexical_queries.iter().any(
            |query| query.match_query == "\"alpha\" AND \"timeout\" AND \"root\" AND \"cause\""
        ));
    }

    #[test]
    fn query_plan_keeps_file_intent_terms_in_short_queries() {
        let plan = build_query_plan("settings file");

        assert_eq!(plan.focus_query, "settings file");
        assert!(plan
            .lexical_queries
            .iter()
            .any(|query| query.match_query == "\"settings\" AND \"file\""));
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
            &build_query_plan("alpha timeout root cause"),
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
