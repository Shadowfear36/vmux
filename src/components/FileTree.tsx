import React, { useEffect, useState, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useStore } from '../store';
import './FileTree.css';

interface FileEntry {
  name: string;
  path: string;
  is_dir: boolean;
  children?: FileEntry[];
}

export function FileTree() {
  const { toggleFileTree, terminals, focusedTerminalId } = useStore();
  const [root, setRoot] = useState<string | null>(null);
  const [tree, setTree] = useState<FileEntry[]>([]);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  // Use the focused terminal's working dir as root
  useEffect(() => {
    const term = focusedTerminalId ? terminals[focusedTerminalId] : null;
    const dir = term?.working_dir ?? null;
    if (dir && dir !== root) {
      setRoot(dir);
      setExpanded(new Set());
      loadDir(dir).then(setTree);
    }
  }, [focusedTerminalId, terminals]);

  const loadDir = useCallback(async (dirPath: string): Promise<FileEntry[]> => {
    try {
      const entries: FileEntry[] = await invoke('list_directory', { path: dirPath });
      return entries.sort((a, b) => {
        if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
        return a.name.localeCompare(b.name);
      });
    } catch {
      return [];
    }
  }, []);

  const toggleDir = useCallback(async (path: string) => {
    setExpanded(prev => {
      const next = new Set(prev);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  }, []);

  const openFile = useCallback((path: string) => {
    // Write to the focused terminal to open with vim
    const store = useStore.getState();
    if (store.focusedTerminalId) {
      // Escape path for shell
      const escaped = path.replace(/\\/g, '/');
      invoke('write_terminal', {
        terminalId: store.focusedTerminalId,
        data: Array.from(new TextEncoder().encode(`vim "${escaped}"\r`)),
      });
    }
  }, []);

  return (
    <div className="ftree">
      <div className="ftree-header">
        <span className="ftree-title">Files</span>
        <button className="ftree-close" onClick={toggleFileTree}>x</button>
      </div>
      {root && (
        <div className="ftree-root" title={root}>
          {root.split(/[\\/]/).slice(-2).join('/')}
        </div>
      )}
      <div className="ftree-list">
        {tree.map(entry => (
          <FileNode
            key={entry.path}
            entry={entry}
            depth={0}
            expanded={expanded}
            onToggle={toggleDir}
            onOpen={openFile}
            loadDir={loadDir}
          />
        ))}
        {tree.length === 0 && root && (
          <div className="ftree-empty">No files found</div>
        )}
        {!root && (
          <div className="ftree-empty">Focus a terminal to browse files</div>
        )}
      </div>
    </div>
  );
}

interface NodeProps {
  entry: FileEntry;
  depth: number;
  expanded: Set<string>;
  onToggle: (path: string) => void;
  onOpen: (path: string) => void;
  loadDir: (path: string) => Promise<FileEntry[]>;
}

function FileNode({ entry, depth, expanded, onToggle, onOpen, loadDir }: NodeProps) {
  const [children, setChildren] = useState<FileEntry[] | null>(null);
  const isExpanded = expanded.has(entry.path);

  useEffect(() => {
    if (entry.is_dir && isExpanded && children === null) {
      loadDir(entry.path).then(setChildren);
    }
  }, [isExpanded]);

  const indent = depth * 16 + 8;

  if (!entry.is_dir) {
    return (
      <div
        className="ftree-file"
        style={{ paddingLeft: indent }}
        onClick={() => onOpen(entry.path)}
        title={entry.path}
      >
        <span className="ftree-icon">{fileIcon(entry.name)}</span>
        <span className="ftree-name">{entry.name}</span>
      </div>
    );
  }

  return (
    <>
      <div
        className="ftree-dir"
        style={{ paddingLeft: indent }}
        onClick={() => onToggle(entry.path)}
        title={entry.path}
      >
        <span className="ftree-arrow">{isExpanded ? '▾' : '▸'}</span>
        <span className="ftree-name">{entry.name}</span>
      </div>
      {isExpanded && children?.map(child => (
        <FileNode
          key={child.path}
          entry={child}
          depth={depth + 1}
          expanded={expanded}
          onToggle={onToggle}
          onOpen={onOpen}
          loadDir={loadDir}
        />
      ))}
    </>
  );
}

function fileIcon(name: string): string {
  const ext = name.split('.').pop()?.toLowerCase() ?? '';
  switch (ext) {
    case 'rs': return 'R';
    case 'ts': case 'tsx': return 'T';
    case 'js': case 'jsx': return 'J';
    case 'json': return '{';
    case 'toml': case 'yaml': case 'yml': return '*';
    case 'css': return '#';
    case 'html': return '<';
    case 'md': return 'M';
    case 'py': return 'P';
    case 'go': return 'G';
    case 'sql': return 'S';
    default: return '-';
  }
}
