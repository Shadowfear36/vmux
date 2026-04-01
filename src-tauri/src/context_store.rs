/// Context store for managing persistent context between agent sessions.
/// Includes: context entries, projects, conversations, conversation chunks, agent configs.
use serde::{Deserialize, Serialize};
use rusqlite::{Connection, params};
use uuid::Uuid;
use anyhow::Result;

// ─── Context Entries (existing) ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextEntry {
    pub id: String,
    pub title: String,
    pub content: String,
    pub workspace_id: Option<String>,
    pub tab_id: Option<String>,
    pub tags: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

// ─── Projects ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
    pub created_at: i64,
    pub updated_at: i64,
}

// ─── Conversations ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub project_id: String,
    pub agent_type: String,
    pub session_id: Option<String>,
    pub title: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub source: String, // "transcript" or "terminal_capture"
    pub metadata: String, // JSON blob
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationChunk {
    pub id: String,
    pub conversation_id: String,
    pub chunk_index: i32,
    pub role: String, // "user", "assistant", "tool_result"
    pub content: String,
    pub has_embedding: bool,
    pub created_at: i64,
}

// ─── Agent Configs ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub content: String,
    pub auto_generated: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

// ─── Search Results ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub chunk: ConversationChunk,
    pub conversation_title: Option<String>,
    pub project_name: String,
    pub project_path: String,
    pub score: f32,
}

// ─── Store ───────────────────────────────────────────────────────────────────

pub struct ContextStore {
    db: Connection,
}

impl ContextStore {
    pub fn new(db_path: &str) -> Result<Self> {
        let db = Connection::open(db_path)?;
        db.execute_batch("
            CREATE TABLE IF NOT EXISTS context_entries (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                workspace_id TEXT,
                tab_id TEXT,
                tags TEXT NOT NULL DEFAULT '[]',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_workspace ON context_entries(workspace_id);
            CREATE INDEX IF NOT EXISTS idx_tab ON context_entries(tab_id);

            CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                path TEXT NOT NULL UNIQUE,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS conversations (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                agent_type TEXT NOT NULL,
                session_id TEXT,
                title TEXT,
                started_at INTEGER NOT NULL,
                ended_at INTEGER,
                source TEXT NOT NULL,
                metadata TEXT NOT NULL DEFAULT '{}',
                FOREIGN KEY (project_id) REFERENCES projects(id)
            );
            CREATE INDEX IF NOT EXISTS idx_conv_project ON conversations(project_id);
            CREATE INDEX IF NOT EXISTS idx_conv_session ON conversations(session_id);

            CREATE TABLE IF NOT EXISTS conversation_chunks (
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                chunk_index INTEGER NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                embedding BLOB,
                created_at INTEGER NOT NULL,
                FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_chunks_conv ON conversation_chunks(conversation_id);

            CREATE TABLE IF NOT EXISTS agent_configs (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                name TEXT NOT NULL DEFAULT 'agent.md',
                content TEXT NOT NULL,
                auto_generated INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                FOREIGN KEY (project_id) REFERENCES projects(id)
            );
            CREATE INDEX IF NOT EXISTS idx_agent_project ON agent_configs(project_id);

            CREATE TABLE IF NOT EXISTS browser_history (
                id TEXT PRIMARY KEY,
                url TEXT NOT NULL,
                title TEXT,
                visited_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_bh_visited ON browser_history(visited_at DESC);
        ")?;
        Ok(ContextStore { db })
    }

    // ── Context Entries (existing CRUD) ──────────────────────────────────────

    pub fn create(&self, title: &str, content: &str, workspace_id: Option<&str>, tab_id: Option<&str>, tags: Vec<String>) -> Result<ContextEntry> {
        let now = chrono_now();
        let entry = ContextEntry {
            id: Uuid::new_v4().to_string(),
            title: title.to_string(),
            content: content.to_string(),
            workspace_id: workspace_id.map(|s| s.to_string()),
            tab_id: tab_id.map(|s| s.to_string()),
            tags,
            created_at: now,
            updated_at: now,
        };
        self.db.execute(
            "INSERT INTO context_entries (id, title, content, workspace_id, tab_id, tags, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                entry.id, entry.title, entry.content, entry.workspace_id, entry.tab_id,
                serde_json::to_string(&entry.tags).unwrap_or_default(),
                entry.created_at, entry.updated_at
            ],
        )?;
        Ok(entry)
    }

    pub fn update(&self, id: &str, title: Option<&str>, content: Option<&str>, tags: Option<Vec<String>>) -> Result<()> {
        let now = chrono_now();
        if let Some(t) = title {
            self.db.execute("UPDATE context_entries SET title=?1, updated_at=?2 WHERE id=?3", params![t, now, id])?;
        }
        if let Some(c) = content {
            self.db.execute("UPDATE context_entries SET content=?1, updated_at=?2 WHERE id=?3", params![c, now, id])?;
        }
        if let Some(t) = tags {
            let tags_json = serde_json::to_string(&t).unwrap_or_default();
            self.db.execute("UPDATE context_entries SET tags=?1, updated_at=?2 WHERE id=?3", params![tags_json, now, id])?;
        }
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        self.db.execute("DELETE FROM context_entries WHERE id=?1", params![id])?;
        Ok(())
    }

    pub fn list(&self, workspace_id: Option<&str>) -> Result<Vec<ContextEntry>> {
        if let Some(wid) = workspace_id {
            let mut stmt = self.db.prepare(
                "SELECT id, title, content, workspace_id, tab_id, tags, created_at, updated_at
                 FROM context_entries WHERE workspace_id=?1 ORDER BY updated_at DESC"
            )?;
            let rows = stmt.query_map(params![wid], row_to_entry)?;
            return Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        let mut stmt = self.db.prepare(
            "SELECT id, title, content, workspace_id, tab_id, tags, created_at, updated_at
             FROM context_entries ORDER BY updated_at DESC"
        )?;
        let rows = stmt.query_map([], row_to_entry)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    #[allow(dead_code)]
    pub fn get(&self, id: &str) -> Result<Option<ContextEntry>> {
        let mut stmt = self.db.prepare(
            "SELECT id, title, content, workspace_id, tab_id, tags, created_at, updated_at
             FROM context_entries WHERE id=?1"
        )?;
        let mut rows = stmt.query_map(params![id], row_to_entry)?;
        Ok(rows.next().transpose()?)
    }

    // ── Projects ─────────────────────────────────────────────────────────────

    /// Register a project by its directory path. Returns existing if already registered.
    pub fn ensure_project(&self, path: &str, name: &str) -> Result<Project> {
        // Check if already exists
        let mut stmt = self.db.prepare("SELECT id, name, path, created_at, updated_at FROM projects WHERE path=?1")?;
        let existing: Option<Project> = stmt.query_map(params![path], |row| {
            Ok(Project { id: row.get(0)?, name: row.get(1)?, path: row.get(2)?, created_at: row.get(3)?, updated_at: row.get(4)? })
        })?.next().transpose()?;

        if let Some(p) = existing {
            return Ok(p);
        }

        let now = chrono_now();
        let project = Project {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            path: path.to_string(),
            created_at: now,
            updated_at: now,
        };
        self.db.execute(
            "INSERT INTO projects (id, name, path, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![project.id, project.name, project.path, project.created_at, project.updated_at],
        )?;
        Ok(project)
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let mut stmt = self.db.prepare("SELECT id, name, path, created_at, updated_at FROM projects ORDER BY updated_at DESC")?;
        let rows = stmt.query_map([], |row| {
            Ok(Project { id: row.get(0)?, name: row.get(1)?, path: row.get(2)?, created_at: row.get(3)?, updated_at: row.get(4)? })
        })?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_project_by_path(&self, path: &str) -> Result<Option<Project>> {
        let mut stmt = self.db.prepare("SELECT id, name, path, created_at, updated_at FROM projects WHERE path=?1")?;
        let mut rows = stmt.query_map(params![path], |row| {
            Ok(Project { id: row.get(0)?, name: row.get(1)?, path: row.get(2)?, created_at: row.get(3)?, updated_at: row.get(4)? })
        })?;
        Ok(rows.next().transpose()?)
    }

    // ── Conversations ────────────────────────────────────────────────────────

    pub fn create_conversation(&self, project_id: &str, agent_type: &str, session_id: Option<&str>,
                                title: Option<&str>, source: &str) -> Result<Conversation> {
        let now = chrono_now();
        let conv = Conversation {
            id: Uuid::new_v4().to_string(),
            project_id: project_id.to_string(),
            agent_type: agent_type.to_string(),
            session_id: session_id.map(|s| s.to_string()),
            title: title.map(|s| s.to_string()),
            started_at: now,
            ended_at: None,
            source: source.to_string(),
            metadata: "{}".to_string(),
        };
        self.db.execute(
            "INSERT INTO conversations (id, project_id, agent_type, session_id, title, started_at, source, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![conv.id, conv.project_id, conv.agent_type, conv.session_id, conv.title, conv.started_at, conv.source, conv.metadata],
        )?;
        Ok(conv)
    }

    pub fn end_conversation(&self, id: &str) -> Result<()> {
        let now = chrono_now();
        self.db.execute("UPDATE conversations SET ended_at=?1 WHERE id=?2", params![now, id])?;
        Ok(())
    }

    pub fn list_conversations(&self, project_id: Option<&str>) -> Result<Vec<Conversation>> {
        if let Some(pid) = project_id {
            let mut stmt = self.db.prepare(
                "SELECT id, project_id, agent_type, session_id, title, started_at, ended_at, source, metadata
                 FROM conversations WHERE project_id=?1 ORDER BY started_at DESC"
            )?;
            let rows = stmt.query_map(params![pid], row_to_conversation)?;
            return Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        let mut stmt = self.db.prepare(
            "SELECT id, project_id, agent_type, session_id, title, started_at, ended_at, source, metadata
             FROM conversations ORDER BY started_at DESC"
        )?;
        let rows = stmt.query_map([], row_to_conversation)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Check if a conversation with this session_id already exists (avoid duplicate imports).
    pub fn conversation_exists_by_session(&self, session_id: &str) -> Result<bool> {
        let count: i64 = self.db.query_row(
            "SELECT COUNT(*) FROM conversations WHERE session_id=?1",
            params![session_id], |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    // ── Conversation Chunks ──────────────────────────────────────────────────

    pub fn add_chunk(&self, conversation_id: &str, chunk_index: i32, role: &str, content: &str) -> Result<ConversationChunk> {
        let now = chrono_now();
        let chunk = ConversationChunk {
            id: Uuid::new_v4().to_string(),
            conversation_id: conversation_id.to_string(),
            chunk_index,
            role: role.to_string(),
            content: content.to_string(),
            has_embedding: false,
            created_at: now,
        };
        self.db.execute(
            "INSERT INTO conversation_chunks (id, conversation_id, chunk_index, role, content, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![chunk.id, chunk.conversation_id, chunk.chunk_index, chunk.role, chunk.content, chunk.created_at],
        )?;
        Ok(chunk)
    }

    pub fn get_chunks(&self, conversation_id: &str) -> Result<Vec<ConversationChunk>> {
        let mut stmt = self.db.prepare(
            "SELECT id, conversation_id, chunk_index, role, content, embedding IS NOT NULL, created_at
             FROM conversation_chunks WHERE conversation_id=?1 ORDER BY chunk_index ASC"
        )?;
        let rows = stmt.query_map(params![conversation_id], row_to_chunk)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Store embedding for a chunk.
    pub fn set_chunk_embedding(&self, chunk_id: &str, embedding: &[f32]) -> Result<()> {
        let bytes: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();
        self.db.execute("UPDATE conversation_chunks SET embedding=?1 WHERE id=?2", params![bytes, chunk_id])?;
        Ok(())
    }

    /// Get all chunks that have embeddings, optionally filtered by project.
    pub fn get_embedded_chunks(&self, project_id: Option<&str>) -> Result<Vec<(ConversationChunk, Vec<f32>)>> {
        let chunk_mapper = |row: &rusqlite::Row| -> rusqlite::Result<(ConversationChunk, Vec<f32>)> {
            let chunk = row_to_chunk(row)?;
            let blob: Vec<u8> = row.get(7)?;
            let embedding = bytes_to_f32(&blob);
            Ok((chunk, embedding))
        };

        if let Some(pid) = project_id {
            let mut stmt = self.db.prepare(
                "SELECT cc.id, cc.conversation_id, cc.chunk_index, cc.role, cc.content, 1, cc.created_at, cc.embedding
                 FROM conversation_chunks cc JOIN conversations c ON cc.conversation_id = c.id
                 WHERE cc.embedding IS NOT NULL AND c.project_id=?1"
            )?;
            let rows = stmt.query_map(params![pid], chunk_mapper)?;
            return Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        let mut stmt = self.db.prepare(
            "SELECT cc.id, cc.conversation_id, cc.chunk_index, cc.role, cc.content, 1, cc.created_at, cc.embedding
             FROM conversation_chunks cc WHERE cc.embedding IS NOT NULL"
        )?;
        let rows = stmt.query_map([], chunk_mapper)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Get chunks without embeddings (for batch embedding).
    pub fn get_unembedded_chunks(&self, limit: usize) -> Result<Vec<ConversationChunk>> {
        let mut stmt = self.db.prepare(
            "SELECT id, conversation_id, chunk_index, role, content, 0, created_at
             FROM conversation_chunks WHERE embedding IS NULL LIMIT ?1"
        )?;
        let rows = stmt.query_map(params![limit as i64], row_to_chunk)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // ── Agent Configs ────────────────────────────────────────────────────────

    pub fn get_agent_config(&self, project_id: &str) -> Result<Option<AgentConfig>> {
        let mut stmt = self.db.prepare(
            "SELECT id, project_id, name, content, auto_generated, created_at, updated_at
             FROM agent_configs WHERE project_id=?1 ORDER BY updated_at DESC LIMIT 1"
        )?;
        let mut rows = stmt.query_map(params![project_id], row_to_agent_config)?;
        Ok(rows.next().transpose()?)
    }

    pub fn save_agent_config(&self, project_id: &str, name: &str, content: &str, auto_generated: bool) -> Result<AgentConfig> {
        let now = chrono_now();
        // Upsert: update if exists for this project, insert if not
        let existing = self.get_agent_config(project_id)?;
        if let Some(existing) = existing {
            self.db.execute(
                "UPDATE agent_configs SET content=?1, name=?2, auto_generated=?3, updated_at=?4 WHERE id=?5",
                params![content, name, auto_generated as i32, now, existing.id],
            )?;
            Ok(AgentConfig { content: content.to_string(), name: name.to_string(), auto_generated, updated_at: now, ..existing })
        } else {
            let config = AgentConfig {
                id: Uuid::new_v4().to_string(),
                project_id: project_id.to_string(),
                name: name.to_string(),
                content: content.to_string(),
                auto_generated,
                created_at: now,
                updated_at: now,
            };
            self.db.execute(
                "INSERT INTO agent_configs (id, project_id, name, content, auto_generated, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![config.id, config.project_id, config.name, config.content, config.auto_generated as i32, config.created_at, config.updated_at],
            )?;
            Ok(config)
        }
    }

    pub fn export_agent_config(&self, project_id: &str, output_path: &str) -> Result<()> {
        if let Some(config) = self.get_agent_config(project_id)? {
            std::fs::write(output_path, &config.content)?;
        }
        Ok(())
    }

    // ── Accessors for search results ─────────────────────────────────────────

    pub fn get_conversation_meta(&self, conversation_id: &str) -> Result<Option<(String, String, String)>> {
        let mut stmt = self.db.prepare(
            "SELECT c.title, p.name, p.path FROM conversations c
             JOIN projects p ON c.project_id = p.id
             WHERE c.id=?1"
        )?;
        let mut rows = stmt.query_map(params![conversation_id], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        Ok(rows.next().transpose()?)
    }

    // ── Browser History ──────────────────────────────────────────────────────

    pub fn add_history(&self, url: &str, title: Option<&str>) -> Result<()> {
        let id = Uuid::new_v4().to_string();
        let now = chrono_now();
        self.db.execute(
            "INSERT INTO browser_history (id, url, title, visited_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, url, title, now],
        )?;
        Ok(())
    }

    pub fn list_history(&self, limit: usize) -> Result<Vec<BrowserHistoryEntry>> {
        let mut stmt = self.db.prepare(
            "SELECT id, url, title, visited_at FROM browser_history ORDER BY visited_at DESC LIMIT ?1"
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(BrowserHistoryEntry {
                id: row.get(0)?, url: row.get(1)?, title: row.get(2)?, visited_at: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn clear_history(&self) -> Result<()> {
        self.db.execute("DELETE FROM browser_history", [])?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserHistoryEntry {
    pub id: String,
    pub url: String,
    pub title: Option<String>,
    pub visited_at: i64,
}

// ─── Row Mappers ─────────────────────────────────────────────────────────────

fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<ContextEntry> {
    let tags_str: String = row.get(5)?;
    let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();
    Ok(ContextEntry {
        id: row.get(0)?, title: row.get(1)?, content: row.get(2)?,
        workspace_id: row.get(3)?, tab_id: row.get(4)?, tags,
        created_at: row.get(6)?, updated_at: row.get(7)?,
    })
}

fn row_to_conversation(row: &rusqlite::Row) -> rusqlite::Result<Conversation> {
    Ok(Conversation {
        id: row.get(0)?, project_id: row.get(1)?, agent_type: row.get(2)?,
        session_id: row.get(3)?, title: row.get(4)?, started_at: row.get(5)?,
        ended_at: row.get(6)?, source: row.get(7)?, metadata: row.get(8)?,
    })
}

fn row_to_chunk(row: &rusqlite::Row) -> rusqlite::Result<ConversationChunk> {
    Ok(ConversationChunk {
        id: row.get(0)?, conversation_id: row.get(1)?, chunk_index: row.get(2)?,
        role: row.get(3)?, content: row.get(4)?, has_embedding: row.get(5)?,
        created_at: row.get(6)?,
    })
}

fn row_to_agent_config(row: &rusqlite::Row) -> rusqlite::Result<AgentConfig> {
    Ok(AgentConfig {
        id: row.get(0)?, project_id: row.get(1)?, name: row.get(2)?,
        content: row.get(3)?, auto_generated: row.get::<_, i32>(4)? != 0,
        created_at: row.get(5)?, updated_at: row.get(6)?,
    })
}

fn bytes_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
