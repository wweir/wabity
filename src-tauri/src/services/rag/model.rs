use std::{
    path::PathBuf,
    sync::{atomic::AtomicU64, Arc},
    time::Duration,
};

use globset::GlobSet;
use tokio::sync::{Mutex as AsyncMutex, RwLock as AsyncRwLock};

use crate::{
    domain::{
        rag::RagRuntimeStatus,
        settings::{LlmProviderConfig, RagSettings},
    },
    services::document_extract::DocumentKind,
};

pub(crate) const RAG_DB_DIR_NAME: &str = "rag-lancedb";
pub(crate) const RAG_METADATA_DB_FILE_NAME: &str = "rag-metadata.sqlite3";
pub(crate) const RAG_TABLE_NAME: &str = "chunks";
pub(crate) const RAG_LEXICAL_TABLE_NAME: &str = "rag_chunk_fts";
pub(crate) const MAX_TEXT_FILE_BYTES: u64 = 50 * 1024 * 1024;
pub(crate) const CHUNK_MAX_CHARS: usize = 1_200;
pub(crate) const CHUNK_OVERLAP_CHARS: usize = 200;
pub(crate) const MARKDOWN_CHUNK_TARGET_CHARS: usize = 350;
pub(crate) const MARKDOWN_CHUNK_HARD_MAX_CHARS: usize = 550;
pub(crate) const MARKDOWN_CHUNK_OVERLAP_CHARS: usize = 80;
pub(crate) const EMBEDDING_BATCH_SIZE_MIN: usize = 1;
pub(crate) const EMBEDDING_BATCH_SIZE_DEFAULT: usize = 8;
pub(crate) const EMBEDDING_BATCH_SIZE_MAX: usize = 128;
pub(crate) const EMBEDDING_BATCH_CHAR_BUDGET: usize =
    CHUNK_MAX_CHARS * EMBEDDING_BATCH_SIZE_DEFAULT;
pub(crate) const EMBEDDING_BATCH_GROWTH_SUCCESS_STREAK: usize = 3;
pub(crate) const EMBEDDING_BATCH_GROWTH_DIVISOR: usize = 4;
pub(crate) const EMBEDDING_BATCH_COOLDOWN_ROUNDS: usize = 2;
pub(crate) const EMBEDDING_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
pub(crate) const WATCH_DEBOUNCE_WINDOW: Duration = Duration::from_millis(250);
pub(crate) const MAX_DELETE_FILTER_PATHS: usize = 128;
pub(crate) const MAX_METADATA_BATCH_PATHS: usize = 256;
pub(crate) const MAX_TEXT_FINGERPRINT_FILTERS: usize = 256;
pub(crate) const MAX_STREAMING_REINDEX_CONCURRENCY: usize = 4;
pub(crate) const VECTOR_INDEX_REBUILD_MIN_DIRTY_CHUNKS: usize = 256;
pub(crate) const VECTOR_INDEX_REBUILD_MIN_DIRTY_DELETES: usize = 8;

#[derive(Debug, Clone)]
pub(crate) struct RagChunk {
    pub(crate) id: String,
    pub(crate) source_root: String,
    pub(crate) absolute_path: String,
    pub(crate) version_id: String,
    pub(crate) embedding_fingerprint: String,
    pub(crate) document_kind: DocumentKind,
    pub(crate) chunk_state: RagChunkState,
    pub(crate) chunk_index: i32,
    pub(crate) line_start: Option<i32>,
    pub(crate) line_end: Option<i32>,
    pub(crate) paragraph_line_start: Option<i32>,
    pub(crate) page_start: Option<i32>,
    pub(crate) page_end: Option<i32>,
    pub(crate) heading_path: Vec<String>,
    pub(crate) anchor_label: Option<String>,
    pub(crate) chunk_reuse_key: String,
    pub(crate) text_fingerprint: String,
    pub(crate) text: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedRagConfig {
    pub(crate) source_roots: Vec<PathBuf>,
    pub(crate) ignore_globs: Arc<Option<GlobSet>>,
    pub(crate) embedding_fingerprint: String,
    pub(crate) provider: LlmProviderConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RagRuntimeInputs {
    pub(crate) settings: RagSettings,
    pub(crate) embedding_provider: Option<LlmProviderConfig>,
}

#[derive(Clone)]
pub(crate) struct RagRuntimeContext {
    pub(crate) runtime_status: Arc<AsyncRwLock<RagRuntimeStatus>>,
    pub(crate) runtime_generation: Arc<AtomicU64>,
    pub(crate) storage_lock: Arc<AsyncMutex<()>>,
    pub(crate) generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RagRuntimeStartMode {
    ReuseIndex,
    RebuildIndex,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EmbeddingTargetIdentity {
    StableModel {
        namespace: &'static str,
        model_identity: String,
    },
    EndpointBound {
        normalized_base_url: String,
        model_identity: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RagIndexedFileRecord {
    pub(crate) source_root: String,
    pub(crate) absolute_path: String,
    pub(crate) relative_path: String,
    pub(crate) embedding_fingerprint: String,
    pub(crate) extractor_fingerprint: String,
    pub(crate) active: Option<RagIndexedFileVersion>,
    pub(crate) pending: Option<RagIndexedFileVersion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RagIndexedFileVersion {
    pub(crate) version_id: String,
    pub(crate) content_md5: String,
    pub(crate) modified_at_ms: Option<i64>,
    pub(crate) size_bytes: i64,
    pub(crate) chunk_count: i64,
    pub(crate) indexed_at_ms: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedRagFile {
    pub(crate) record: RagIndexedFileRecord,
    pub(crate) chunks: Vec<PreparedRagChunk>,
    pub(crate) version_id: String,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedRagChunk {
    pub(crate) document_kind: DocumentKind,
    pub(crate) chunk_index: i32,
    pub(crate) line_start: Option<i32>,
    pub(crate) line_end: Option<i32>,
    pub(crate) paragraph_line_start: Option<i32>,
    pub(crate) page_start: Option<i32>,
    pub(crate) page_end: Option<i32>,
    pub(crate) heading_path: Vec<String>,
    pub(crate) anchor_label: Option<String>,
    pub(crate) chunk_reuse_key: String,
    pub(crate) text: String,
}

#[derive(Debug, Clone)]
pub(crate) struct DocumentExcerpt {
    pub(crate) document_kind: DocumentKind,
    pub(crate) chunk_index: i32,
    pub(crate) text: String,
    pub(crate) line_start: Option<i32>,
    pub(crate) line_end: Option<i32>,
    pub(crate) paragraph_line_start: Option<i32>,
    pub(crate) page_start: Option<i32>,
    pub(crate) page_end: Option<i32>,
    pub(crate) heading_path: Vec<String>,
    pub(crate) anchor_label: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct RagLexicalSearchHit {
    pub(crate) source_root: String,
    pub(crate) absolute_path: String,
    pub(crate) document_kind: DocumentKind,
    pub(crate) chunk_index: i32,
    pub(crate) line_start: Option<i32>,
    pub(crate) line_end: Option<i32>,
    pub(crate) paragraph_line_start: Option<i32>,
    pub(crate) page_start: Option<i32>,
    pub(crate) page_end: Option<i32>,
    pub(crate) heading_path: Vec<String>,
    pub(crate) anchor_label: Option<String>,
    pub(crate) text: String,
    pub(crate) bm25_rank: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RagChunkState {
    Staged,
    Active,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RuntimeProgress {
    pub(crate) scanned_file_count: usize,
    pub(crate) completed_file_count: usize,
    pub(crate) total_file_count: usize,
    pub(crate) pending_file_count: usize,
}

impl RagIndexedFileRecord {
    pub(crate) fn current_chunk_count(&self) -> usize {
        self.active
            .as_ref()
            .map(|version| version.chunk_count.max(0) as usize)
            .unwrap_or_default()
    }

    pub(crate) fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub(crate) fn refresh_active_metadata(
        &self,
        modified_at_ms: Option<i64>,
        size_bytes: i64,
    ) -> Self {
        let mut refreshed = self.clone();
        if let Some(active) = refreshed.active.as_mut() {
            active.modified_at_ms = modified_at_ms;
            active.size_bytes = size_bytes;
        }
        refreshed.pending = None;
        refreshed
    }
}

impl RagChunkState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Staged => "staged",
            Self::Active => "active",
        }
    }
}

impl DocumentKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::PlainText => "plain_text",
            Self::Markdown => "markdown",
            Self::Pdf => "pdf",
            Self::Docx => "docx",
        }
    }
}
