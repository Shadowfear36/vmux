/// RAG (Retrieval-Augmented Generation) search across conversation chunks.
/// Search is split into async (embedding) and sync (DB) phases to avoid
/// holding rusqlite::Connection across await points.

use anyhow::Result;

use crate::context_store::{ContextStore, SearchResult};
use crate::embeddings::{self, EmbeddingConfig};

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

/// Full embed-then-search flow, shared by the `rag_search` Tauri command and
/// the IPC `ContextSearch` handler (so an agent can `vmuxctl context search`
/// the same history the Search tab searches). Embeds the query (async,
/// network), then opens its own `ContextStore` connection on a blocking task
/// to do the DB read — `rusqlite::Connection` isn't `Send`-safe across
/// `.await`, so this can't just reuse a connection held on `AppState`.
pub async fn run_search(
    db_path: String,
    config: EmbeddingConfig,
    query: String,
    project_id: Option<String>,
    top_k: usize,
) -> Result<Vec<SearchResult>, String> {
    let provider = embeddings::create_provider(&config);
    let query_embeddings = provider.embed(&[query])
        .await.map_err(|e| e.to_string())?;
    let query_vec = query_embeddings.into_iter().next()
        .ok_or("empty embedding result")?;

    tokio::task::spawn_blocking(move || {
        let store = ContextStore::new(&db_path).map_err(|e| e.to_string())?;
        search_with_embedding(&store, &query_vec, project_id.as_deref(), top_k)
            .map_err(|e| e.to_string())
    }).await.map_err(|e| e.to_string())?
}
