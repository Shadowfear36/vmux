/// RAG (Retrieval-Augmented Generation) search across conversation chunks.
/// Search is split into async (embedding) and sync (DB) phases to avoid
/// holding rusqlite::Connection across await points.

use anyhow::Result;

use crate::context_store::{ContextStore, SearchResult};
use crate::embeddings;

/// Search using a pre-computed query embedding vector (sync, no async).
pub fn search_with_embedding(
    store: &ContextStore,
    query_vec: &[f32],
    project_id: Option<&str>,
    top_k: usize,
) -> Result<Vec<SearchResult>> {
    let embedded = store.get_embedded_chunks(project_id)?;

    let mut scored: Vec<(f32, _)> = embedded.iter()
        .map(|(chunk, embedding)| {
            let score = embeddings::cosine_similarity(query_vec, embedding);
            (score, chunk)
        })
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(top_k);

    let results: Vec<SearchResult> = scored.iter()
        .filter_map(|(score, chunk)| {
            let meta = store.get_conversation_meta(&chunk.conversation_id).ok()??;
            Some(SearchResult {
                chunk: (*chunk).clone(),
                conversation_title: if meta.0.is_empty() { None } else { Some(meta.0) },
                project_name: meta.1,
                project_path: meta.2,
                score: *score,
            })
        })
        .collect();

    Ok(results)
}
