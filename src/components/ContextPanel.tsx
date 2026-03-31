import React, { useState, useEffect } from 'react';
import { useStore } from '../store';
import type { ContextEntry } from '../types';
import './ContextPanel.css';

export function ContextPanel() {
  const {
    contextEntries, activeWorkspaceId,
    loadContext, createContext, updateContextEntry, deleteContext,
    toggleContext,
  } = useStore();

  const [editingId, setEditingId] = useState<string | null>(null);
  const [showNew, setShowNew] = useState(false);
  const [newTitle, setNewTitle] = useState('');
  const [newContent, setNewContent] = useState('');
  const [newTags, setNewTags] = useState('');
  const [search, setSearch] = useState('');
  const [expandedId, setExpandedId] = useState<string | null>(null);

  useEffect(() => {
    loadContext(activeWorkspaceId ?? undefined);
  }, [activeWorkspaceId]);

  const filtered = contextEntries.filter(e => {
    if (!search) return true;
    const q = search.toLowerCase();
    return (
      e.title.toLowerCase().includes(q) ||
      e.content.toLowerCase().includes(q) ||
      e.tags.some(t => t.toLowerCase().includes(q))
    );
  });

  const handleCreate = async () => {
    if (!newTitle.trim() && !newContent.trim()) return;
    const tags = newTags.split(',').map(t => t.trim()).filter(Boolean);
    await createContext(
      newTitle.trim() || 'Untitled',
      newContent.trim(),
      tags,
    );
    setNewTitle('');
    setNewContent('');
    setNewTags('');
    setShowNew(false);
  };

  const formatDate = (ts: number) => {
    const d = new Date(ts * 1000);
    return d.toLocaleDateString() + ' ' + d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  };

  const copyToClipboard = (text: string) => {
    navigator.clipboard.writeText(text);
  };

  return (
    <div className="ctx-panel">
      <div className="ctx-header">
        <span className="ctx-title">Context Manager</span>
        <button className="ctx-close" onClick={toggleContext}>x</button>
      </div>

      <div className="ctx-toolbar">
        <input
          className="ctx-search"
          placeholder="Search contexts..."
          value={search}
          onChange={e => setSearch(e.target.value)}
        />
        <button className="ctx-new-btn" onClick={() => setShowNew(s => !s)}>
          {showNew ? 'Cancel' : '+ New'}
        </button>
      </div>

      {showNew && (
        <div className="ctx-new-form">
          <input
            className="ctx-input"
            placeholder="Title (e.g. Claude session ID)"
            value={newTitle}
            onChange={e => setNewTitle(e.target.value)}
            autoFocus
          />
          <textarea
            className="ctx-textarea"
            placeholder="Content (e.g. claude --continue abc123, or paste session resume command)"
            rows={4}
            value={newContent}
            onChange={e => setNewContent(e.target.value)}
          />
          <input
            className="ctx-input"
            placeholder="Tags (comma-separated: claude, session, ...)"
            value={newTags}
            onChange={e => setNewTags(e.target.value)}
          />
          <button className="ctx-save-btn" onClick={handleCreate}>Save Context</button>
        </div>
      )}

      <div className="ctx-list">
        {filtered.length === 0 && (
          <div className="ctx-empty">
            {contextEntries.length === 0
              ? 'No contexts yet. Save agent session IDs, notes, or continue commands here.'
              : 'No matches found.'}
          </div>
        )}

        {filtered.map(entry => (
          <ContextEntryCard
            key={entry.id}
            entry={entry}
            expanded={expandedId === entry.id}
            onToggle={() => setExpandedId(expandedId === entry.id ? null : entry.id)}
            onCopy={() => copyToClipboard(entry.content)}
            onDelete={() => deleteContext(entry.id)}
            onUpdate={updateContextEntry}
            formatDate={formatDate}
          />
        ))}
      </div>
    </div>
  );
}

interface CardProps {
  entry: ContextEntry;
  expanded: boolean;
  onToggle: () => void;
  onCopy: () => void;
  onDelete: () => void;
  onUpdate: (id: string, title?: string, content?: string, tags?: string[]) => Promise<void>;
  formatDate: (ts: number) => string;
}

function ContextEntryCard({ entry, expanded, onToggle, onCopy, onDelete, onUpdate, formatDate }: CardProps) {
  const [editing, setEditing] = useState(false);
  const [editTitle, setEditTitle] = useState(entry.title);
  const [editContent, setEditContent] = useState(entry.content);
  const [editTags, setEditTags] = useState(entry.tags.join(', '));

  const handleSave = async () => {
    const tags = editTags.split(',').map(t => t.trim()).filter(Boolean);
    await onUpdate(entry.id, editTitle, editContent, tags);
    setEditing(false);
  };

  return (
    <div className={`ctx-entry ${expanded ? 'ctx-entry-expanded' : ''}`}>
      <div className="ctx-entry-header" onClick={onToggle}>
        <span className="ctx-entry-arrow">{expanded ? '▾' : '▸'}</span>
        <span className="ctx-entry-title">{entry.title}</span>
        <div className="ctx-entry-tags">
          {entry.tags.map(t => (
            <span key={t} className="ctx-tag">{t}</span>
          ))}
        </div>
        <span className="ctx-entry-date">{formatDate(entry.updated_at)}</span>
      </div>

      {expanded && (
        <div className="ctx-entry-body">
          {editing ? (
            <div className="ctx-edit-form">
              <input
                className="ctx-input"
                value={editTitle}
                onChange={e => setEditTitle(e.target.value)}
              />
              <textarea
                className="ctx-textarea"
                rows={4}
                value={editContent}
                onChange={e => setEditContent(e.target.value)}
              />
              <input
                className="ctx-input"
                placeholder="Tags"
                value={editTags}
                onChange={e => setEditTags(e.target.value)}
              />
              <div className="ctx-edit-actions">
                <button className="ctx-save-btn" onClick={handleSave}>Save</button>
                <button className="ctx-cancel-btn" onClick={() => setEditing(false)}>Cancel</button>
              </div>
            </div>
          ) : (
            <>
              <pre className="ctx-entry-content">{entry.content}</pre>
              <div className="ctx-entry-actions">
                <button className="ctx-action-btn" onClick={onCopy} title="Copy to clipboard">Copy</button>
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
