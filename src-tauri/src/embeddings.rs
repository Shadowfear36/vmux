/// Embedding system for semantic search across conversation chunks.
/// Supports Voyage AI (via Anthropic partnership) and a local TF-IDF fallback.

use anyhow::Result;
use serde::{Deserialize, Serialize};

// ─── Embedding Provider Trait ────────────────────────────────────────────────

#[async_trait::async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    fn dimension(&self) -> usize;
}

// ─── Voyage AI Provider ──────────────────────────────────────────────────────

pub struct VoyageProvider {
    api_key: String,
    model: String,
}

#[derive(Serialize)]
struct VoyageRequest {
    input: Vec<String>,
    model: String,
}

#[derive(Deserialize)]
struct VoyageResponse {
    data: Vec<VoyageEmbedding>,
}

#[derive(Deserialize)]
struct VoyageEmbedding {
    embedding: Vec<f32>,
}

impl VoyageProvider {
    pub fn new(api_key: String) -> Self {
        VoyageProvider {
            api_key,
            model: "voyage-code-3".to_string(),
        }
    }
}

#[async_trait::async_trait]
impl EmbeddingProvider for VoyageProvider {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let client = reqwest::Client::new();
        let resp = client
            .post("https://api.voyageai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&VoyageRequest {
                input: texts.to_vec(),
                model: self.model.clone(),
            })
            .send()
            .await?
            .json::<VoyageResponse>()
            .await?;

        Ok(resp.data.into_iter().map(|e| e.embedding).collect())
    }

    fn dimension(&self) -> usize { 1024 }
}

// ─── OpenAI-compatible Provider (works with many APIs) ───────────────────────

pub struct OpenAICompatProvider {
    api_key: String,
    base_url: String,
    model: String,
}

#[derive(Serialize)]
struct OpenAIEmbedRequest {
    input: Vec<String>,
    model: String,
}

#[derive(Deserialize)]
struct OpenAIEmbedResponse {
    data: Vec<OpenAIEmbedData>,
}

#[derive(Deserialize)]
struct OpenAIEmbedData {
    embedding: Vec<f32>,
}

impl OpenAICompatProvider {
    pub fn new(api_key: String, base_url: String, model: String) -> Self {
        OpenAICompatProvider { api_key, base_url, model }
    }
}

#[async_trait::async_trait]
impl EmbeddingProvider for OpenAICompatProvider {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let client = reqwest::Client::new();
        let url = format!("{}/embeddings", self.base_url.trim_end_matches('/'));
        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&OpenAIEmbedRequest {
                input: texts.to_vec(),
                model: self.model.clone(),
            })
            .send()
            .await?
            .json::<OpenAIEmbedResponse>()
            .await?;

        Ok(resp.data.into_iter().map(|e| e.embedding).collect())
    }

    fn dimension(&self) -> usize { 1536 } // typical for text-embedding-3-small
}

// ─── Local BM25-style fallback (no API needed) ──────────────────────────────

pub struct LocalProvider;

impl LocalProvider {
    pub fn new() -> Self { LocalProvider }

    /// Simple term-frequency vector. Not true embeddings but allows
    /// cosine similarity scoring without any external API.
    fn term_vector(text: &str) -> Vec<(String, f32)> {
        let mut counts: std::collections::HashMap<String, f32> = std::collections::HashMap::new();
        for word in text.split_whitespace() {
            let normalized = word.to_lowercase()
                .chars().filter(|c| c.is_alphanumeric()).collect::<String>();
            if normalized.len() >= 2 {
                *counts.entry(normalized).or_default() += 1.0;
            }
        }
        let total = counts.values().sum::<f32>().max(1.0);
        counts.into_iter().map(|(k, v)| (k, v / total)).collect()
    }

    /// Convert term vector to a fixed-size hash-based embedding.
    fn hash_embed(text: &str, dim: usize) -> Vec<f32> {
        let terms = Self::term_vector(text);
        let mut vec = vec![0.0f32; dim];
        for (term, weight) in &terms {
            let hash = term.bytes().fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
            let idx = (hash as usize) % dim;
            vec[idx] += weight;
            // Second hash for less collision
            let idx2 = ((hash >> 16) as usize) % dim;
            vec[idx2] += weight * 0.5;
        }
        // Normalize
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-8);
        vec.iter_mut().for_each(|x| *x /= norm);
        vec
    }
}

#[async_trait::async_trait]
impl EmbeddingProvider for LocalProvider {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| Self::hash_embed(t, 256)).collect())
    }

    fn dimension(&self) -> usize { 256 }
}

// ─── Cosine Similarity ──────────────────────────────────────────────────────

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() { return 0.0; }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a < 1e-8 || norm_b < 1e-8 { return 0.0; }
    dot / (norm_a * norm_b)
}

// ─── Provider Factory ────────────────────────────────────────────────────────

pub fn create_provider(config: &EmbeddingConfig) -> Box<dyn EmbeddingProvider> {
    match config.provider.as_str() {
        "voyage" => {
            Box::new(VoyageProvider::new(config.api_key.clone()))
        }
        "openai" => {
            Box::new(OpenAICompatProvider::new(
                config.api_key.clone(),
                config.base_url.clone().unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
                config.model.clone().unwrap_or_else(|| "text-embedding-3-small".to_string()),
            ))
        }
        _ => Box::new(LocalProvider::new()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    pub provider: String, // "voyage", "openai", "local"
    pub api_key: String,
    pub base_url: Option<String>,
    pub model: Option<String>,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        EmbeddingConfig {
            provider: "local".to_string(),
            api_key: String::new(),
            base_url: None,
            model: None,
        }
    }
}
