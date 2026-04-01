import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useStore } from '../store';
import type { ContextEntry, Project, Conversation, ConversationChunk, AgentConfig } from '../types';
import './ContextPanel.css';

type Tab = 'context' | 'conversations' | 'agent' | 'search';

export function ContextPanel() {
  const [activeTab, setActiveTab] = useState<Tab>('context');
  const { toggleContext } = useStore();

  return (
    <div className="ctx-panel">
      <div className="ctx-header">
        <span className="ctx-title">Context Manager</span>
        <button className="ctx-close" onClick={toggleContext}>x</button>
      </div>
      <div className="ctx-tabs">
        {(['context', 'conversations', 'agent', 'search'] as Tab[]).map(tab => (
          <button
            key={tab}
            className={`ctx-tab ${activeTab === tab ? 'ctx-tab-active' : ''}`}
            onClick={() => setActiveTab(tab)}
          >
            {tab === 'context' ? 'Notes' : tab === 'conversations' ? 'History' : tab === 'agent' ? 'Agent.md' : 'Search'}
          </button>
        ))}
      </div>
      {activeTab === 'context' && <ContextTab />}
      {activeTab === 'conversations' && <ConversationsTab />}
      {activeTab === 'agent' && <AgentConfigTab />}
      {activeTab === 'search' && <SearchTab />}
    </div>
  );
}

// ─── Notes Tab (existing context entries) ────────────────────────────────────

function ContextTab() {
  const {
    contextEntries, activeWorkspaceId,
    loadContext, createContext, updateContextEntry, deleteContext,
  } = useStore();

  const [showNew, setShowNew] = useState(false);
  const [newTitle, setNewTitle] = useState('');
  const [newContent, setNewContent] = useState('');
  const [newTags, setNewTags] = useState('');
  const [search, setSearch] = useState('');
  const [expandedId, setExpandedId] = useState<string | null>(null);

  useEffect(() => { loadContext(activeWorkspaceId ?? undefined); }, [activeWorkspaceId]);

  const filtered = contextEntries.filter(e => {
    if (!search) return true;
    const q = search.toLowerCase();
    return e.title.toLowerCase().includes(q) || e.content.toLowerCase().includes(q) || e.tags.some(t => t.toLowerCase().includes(q));
  });

  const handleCreate = async () => {
    if (!newTitle.trim() && !newContent.trim()) return;
    const tags = newTags.split(',').map(t => t.trim()).filter(Boolean);
    await createContext(newTitle.trim() || 'Untitled', newContent.trim(), tags);
    setNewTitle(''); setNewContent(''); setNewTags(''); setShowNew(false);
  };

  return (
    <>
      <div className="ctx-toolbar">
        <input className="ctx-search" placeholder="Search notes..." value={search} onChange={e => setSearch(e.target.value)} />
        <button className="ctx-new-btn" onClick={() => setShowNew(s => !s)}>{showNew ? 'Cancel' : '+ New'}</button>
      </div>
      {showNew && (
        <div className="ctx-new-form">
          <input className="ctx-input" placeholder="Title" value={newTitle} onChange={e => setNewTitle(e.target.value)} autoFocus />
          <textarea className="ctx-textarea" placeholder="Content" rows={4} value={newContent} onChange={e => setNewContent(e.target.value)} />
          <input className="ctx-input" placeholder="Tags (comma-separated)" value={newTags} onChange={e => setNewTags(e.target.value)} />
          <button className="ctx-save-btn" onClick={handleCreate}>Save</button>
        </div>
      )}
      <div className="ctx-list">
        {filtered.length === 0 && <div className="ctx-empty">{contextEntries.length === 0 ? 'No notes yet.' : 'No matches.'}</div>}
        {filtered.map(entry => (
          <ContextEntryCard key={entry.id} entry={entry} expanded={expandedId === entry.id}
            onToggle={() => setExpandedId(expandedId === entry.id ? null : entry.id)}
            onCopy={() => navigator.clipboard.writeText(entry.content)}
            onDelete={() => deleteContext(entry.id)}
            onUpdate={updateContextEntry}
          />
        ))}
      </div>
    </>
  );
}

function ContextEntryCard({ entry, expanded, onToggle, onCopy, onDelete, onUpdate }: {
  entry: ContextEntry; expanded: boolean; onToggle: () => void;
  onCopy: () => void; onDelete: () => void;
  onUpdate: (id: string, title?: string, content?: string, tags?: string[]) => Promise<void>;
}) {
  const [editing, setEditing] = useState(false);
  const [editTitle, setEditTitle] = useState(entry.title);
  const [editContent, setEditContent] = useState(entry.content);
  const [editTags, setEditTags] = useState(entry.tags.join(', '));

  const handleSave = async () => {
    await onUpdate(entry.id, editTitle, editContent, editTags.split(',').map(t => t.trim()).filter(Boolean));
    setEditing(false);
  };

  return (
    <div className={`ctx-entry ${expanded ? 'ctx-entry-expanded' : ''}`}>
      <div className="ctx-entry-header" onClick={onToggle}>
        <span className="ctx-entry-arrow">{expanded ? '▾' : '▸'}</span>
        <span className="ctx-entry-title">{entry.title}</span>
        <div className="ctx-entry-tags">{entry.tags.map(t => <span key={t} className="ctx-tag">{t}</span>)}</div>
      </div>
      {expanded && (
        <div className="ctx-entry-body">
          {editing ? (
            <div className="ctx-edit-form">
              <input className="ctx-input" value={editTitle} onChange={e => setEditTitle(e.target.value)} />
              <textarea className="ctx-textarea" rows={4} value={editContent} onChange={e => setEditContent(e.target.value)} />
              <input className="ctx-input" placeholder="Tags" value={editTags} onChange={e => setEditTags(e.target.value)} />
              <div className="ctx-edit-actions">
                <button className="ctx-save-btn" onClick={handleSave}>Save</button>
                <button className="ctx-cancel-btn" onClick={() => setEditing(false)}>Cancel</button>
              </div>
            </div>
          ) : (
            <>
              <pre className="ctx-entry-content">{entry.content}</pre>
              <div className="ctx-entry-actions">
                <button className="ctx-action-btn" onClick={onCopy}>Copy</button>
                <button className="ctx-action-btn" onClick={() => setEditing(true)}>Edit</button>
                <button className="ctx-action-btn ctx-action-delete" onClick={onDelete}>Delete</button>
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}

// ─── Conversations Tab ───────────────────────────────────────────────────────

function ConversationsTab() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [selectedProject, setSelectedProject] = useState<string | null>(null);
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const [selectedConv, setSelectedConv] = useState<string | null>(null);
  const [chunks, setChunks] = useState<ConversationChunk[]>([]);
  const [importing, setImporting] = useState(false);

  useEffect(() => {
    invoke<Project[]>('list_projects').then(setProjects).catch(() => {});
  }, []);

  useEffect(() => {
    invoke<Conversation[]>('list_conversations', { projectId: selectedProject })
      .then(setConversations).catch(() => {});
  }, [selectedProject]);

  useEffect(() => {
    if (!selectedConv) { setChunks([]); return; }
    invoke<ConversationChunk[]>('get_conversation_chunks', { conversationId: selectedConv })
      .then(setChunks).catch(() => {});
  }, [selectedConv]);

  const handleImport = async () => {
    setImporting(true);
    try {
      const count = await invoke<number>('import_transcripts', { projectPath: null });
      // Refresh
      const p = await invoke<Project[]>('list_projects');
      setProjects(p);
      const c = await invoke<Conversation[]>('list_conversations', { projectId: selectedProject });
      setConversations(c);
      alert(`Imported ${count} chunks from Claude transcripts`);
    } catch (e) {
      console.error(e);
    }
    setImporting(false);
  };

  const formatDate = (ts: number) => new Date(ts * 1000).toLocaleDateString();

  return (
    <>
      <div className="ctx-toolbar">
        <select className="ctx-select" value={selectedProject ?? ''} onChange={e => { setSelectedProject(e.target.value || null); setSelectedConv(null); }}>
          <option value="">All projects</option>
          {projects.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}
        </select>
        <button className="ctx-new-btn" onClick={handleImport} disabled={importing}>
          {importing ? 'Importing...' : 'Import Claude'}
        </button>
      </div>

      {selectedConv ? (
        <div className="ctx-list">
          <button className="ctx-back-btn" onClick={() => setSelectedConv(null)}>Back to list</button>
          <div className="ctx-conv-chunks">
            {chunks.map(chunk => (
              <div key={chunk.id} className={`ctx-chunk ctx-chunk-${chunk.role}`}>
                <div className="ctx-chunk-role">{chunk.role}</div>
                <pre className="ctx-chunk-content">{chunk.content}</pre>
              </div>
            ))}
            {chunks.length === 0 && <div className="ctx-empty">No messages in this conversation.</div>}
          </div>
        </div>
      ) : (
        <div className="ctx-list">
          {conversations.length === 0 && <div className="ctx-empty">No conversations yet. Click "Import Claude" to import transcripts.</div>}
          {conversations.map(conv => (
            <div key={conv.id} className="ctx-entry" onClick={() => setSelectedConv(conv.id)}>
              <div className="ctx-entry-header">
                <span className="ctx-entry-arrow">▸</span>
                <span className="ctx-entry-title">{conv.title ?? 'Untitled session'}</span>
                <span className="ctx-tag">{conv.agent_type}</span>
                <span className="ctx-tag">{conv.source}</span>
                <span className="ctx-entry-date">{formatDate(conv.started_at)}</span>
              </div>
            </div>
          ))}
        </div>
      )}
    </>
  );
}

// ─── Agent.md Tab ────────────────────────────────────────────────────────────

function AgentConfigTab() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [selectedProject, setSelectedProject] = useState<string | null>(null);
  const [config, setConfig] = useState<AgentConfig | null>(null);
  const [editContent, setEditContent] = useState('');
  const [dirty, setDirty] = useState(false);

  useEffect(() => {
    invoke<Project[]>('list_projects').then(setProjects).catch(() => {});
  }, []);

  useEffect(() => {
    if (!selectedProject) { setConfig(null); setEditContent(''); return; }
    invoke<AgentConfig | null>('get_agent_config', { projectId: selectedProject })
      .then(c => { setConfig(c); setEditContent(c?.content ?? ''); setDirty(false); })
      .catch(() => {});
  }, [selectedProject]);

  const handleSave = async () => {
    if (!selectedProject) return;
    const saved = await invoke<AgentConfig>('save_agent_config', {
      projectId: selectedProject, name: 'agent.md', content: editContent, autoGenerated: false,
    });
    setConfig(saved); setDirty(false);
  };

  const handleExport = async () => {
    if (!selectedProject) return;
    const project = projects.find(p => p.id === selectedProject);
    if (!project) return;
    const path = `${project.path}/agent.md`;
    await invoke('export_agent_config', { projectId: selectedProject, outputPath: path });
    alert(`Exported to ${path}`);
  };

  return (
    <>
      <div className="ctx-toolbar">
        <select className="ctx-select" value={selectedProject ?? ''} onChange={e => setSelectedProject(e.target.value || null)}>
          <option value="">Select project...</option>
          {projects.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}
        </select>
      </div>
      {selectedProject ? (
        <div className="ctx-agent-editor">
          <textarea
            className="ctx-agent-textarea"
            value={editContent}
            onChange={e => { setEditContent(e.target.value); setDirty(true); }}
            placeholder="# Agent.md&#10;&#10;Instructions for AI agents working on this project..."
          />
          <div className="ctx-agent-actions">
            <button className="ctx-save-btn" onClick={handleSave} disabled={!dirty}>
              {dirty ? 'Save' : 'Saved'}
            </button>
            <button className="ctx-action-btn" onClick={handleExport}>Export to disk</button>
          </div>
        </div>
      ) : (
        <div className="ctx-empty">Select a project to edit its agent.md configuration.</div>
      )}
    </>
  );
}

// ─── Search Tab (RAG) ────────────────────────────────────────────────────────

interface SearchResultItem {
  chunk: ConversationChunk;
  conversation_title: string | null;
  project_name: string;
  project_path: string;
  score: number;
}

function SearchTab() {
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<SearchResultItem[]>([]);
  const [searching, setSearching] = useState(false);
  const [embedding, setEmbedding] = useState(false);

  const handleSearch = async () => {
    if (!query.trim()) return;
    setSearching(true);
    try {
      const r = await invoke<SearchResultItem[]>('rag_search', { query: query.trim(), projectId: null, topK: 20 });
      setResults(r);
    } catch (e) {
      console.error('Search failed:', e);
    }
    setSearching(false);
  };

  const handleEmbed = async () => {
    setEmbedding(true);
    try {
      const count = await invoke<number>('embed_chunks');
      alert(`Embedded ${count} chunks`);
    } catch (e) {
      console.error('Embedding failed:', e);
    }
    setEmbedding(false);
  };

  return (
    <>
      <div className="ctx-toolbar">
        <input
          className="ctx-search"
          placeholder="Search across all projects..."
          value={query}
          onChange={e => setQuery(e.target.value)}
          onKeyDown={e => e.key === 'Enter' && handleSearch()}
        />
        <button className="ctx-new-btn" onClick={handleSearch} disabled={searching}>
          {searching ? '...' : 'Search'}
        </button>
      </div>
      <div className="ctx-toolbar" style={{ borderTop: 'none', paddingTop: 0 }}>
        <button className="ctx-action-btn" onClick={handleEmbed} disabled={embedding} style={{ fontSize: 10 }}>
          {embedding ? 'Embedding...' : 'Embed all chunks'}
        </button>
      </div>
      <div className="ctx-list">
        {results.length === 0 && !searching && <div className="ctx-empty">Enter a query to search across all conversation history.</div>}
        {results.map((r, i) => (
          <div key={i} className="ctx-entry ctx-entry-expanded">
            <div className="ctx-entry-header">
              <span className="ctx-entry-title">{r.conversation_title ?? 'Untitled'}</span>
              <span className="ctx-tag">{r.project_name}</span>
              <span className="ctx-entry-date">{(r.score * 100).toFixed(0)}%</span>
            </div>
            <div className="ctx-entry-body">
              <pre className="ctx-entry-content">{r.chunk.content.slice(0, 500)}{r.chunk.content.length > 500 ? '...' : ''}</pre>
            </div>
          </div>
        ))}
      </div>
    </>
  );
}
