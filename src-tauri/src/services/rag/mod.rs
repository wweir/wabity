mod chunking;
mod config;
mod embedding;
mod indexing;
mod model;
mod service;
mod status;
mod storage;

pub use service::RagIndexService;

pub(crate) use chunking::load_document_excerpt_for_chunk;
pub(crate) use config::{
    collect_document_access_roots, display_path_for_prompt, parse_document_kind,
    parse_heading_path, path_is_within_roots, rag_database_path, rag_metadata_database_path,
    resolve_embedding_provider,
};
pub(crate) use embedding::{build_embedding_client, request_embeddings};
pub(crate) use model::RAG_TABLE_NAME;
pub(crate) use storage::{metadata_store_has_pending_rows, search_lexical_chunks};

#[cfg(test)]
use indexing::collect_chunks_for_path;

#[cfg(test)]
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(test)]
use anyhow::Result;
#[cfg(test)]
use notify::{event::ModifyKind, Event, EventKind};
#[cfg(test)]
use reqwest::Client as HttpClient;
#[cfg(test)]
use rusqlite::Connection;
#[cfg(test)]
use tokio::sync::{Mutex as AsyncMutex, RwLock as AsyncRwLock};

#[cfg(test)]
use crate::{
    domain::{
        rag::{RagRuntimePhase, RagRuntimeStatus},
        settings::{LlmProviderConfig, LlmSettings, RagSettings},
    },
    services::document_extract::DocumentKind,
};

#[cfg(test)]
use self::{
    chunking::*, config::*, embedding::*, indexing::*, model::*, service::*, status::*, storage::*,
};

#[cfg(test)]
mod tests;
