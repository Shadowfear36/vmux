import React, { useEffect, useRef, useCallback } from 'react';
import { useStore } from '../store';
import type { Tab } from '../types';
import { SplitTreeView, SplitNode, makeLeaf, splitNode, removeNode, getTerminalIds } from './SplitTree';
import './TabView.css';

interface Props {
  tab: Tab;
}

export function TabView({ tab }: Props) {
  const {
    createTerminalInTab, activeWorkspaceId,
    shells, setDefaultShell,
    splitTrees, setSplitTree,
  } = useStore();

  const tree = splitTrees[tab.id] ?? null;

  // Sync tree with pane list: if panes exist but no tree, build one
  useEffect(() => {
    const terminalPanes = tab.panes.filter(p => p.kind.type === 'terminal');
    if (terminalPanes.length === 0) return;

    const existingIds = tree ? getTerminalIds(tree) : [];
    const paneIds = terminalPanes.map(p => p.kind.type === 'terminal' ? p.kind.terminal_id : '').filter(Boolean);

    // If tree already matches panes, nothing to do
    if (existingIds.length === paneIds.length && existingIds.every(id => paneIds.includes(id))) return;

    // No tree yet: try restoring from persisted layout, else build flat
    if (!tree) {
      let restored = false;
      if (tab.layout) {
        try {
          const parsed = JSON.parse(tab.layout) as SplitNode;
          // Validate that the restored tree's terminal IDs match current panes
          const restoredIds = getTerminalIds(parsed);
          if (restoredIds.length === paneIds.length && restoredIds.every(id => paneIds.includes(id))) {
            setSplitTree(tab.id, parsed);
            restored = true;
          }
        } catch { /* invalid JSON, fall through */ }
      }
      if (!restored) {
        let node: SplitNode = makeLeaf(paneIds[0]);
        for (let i = 1; i < paneIds.length; i++) {
          node = {
            type: 'split',
            direction: 'horizontal',
            children: [node, makeLeaf(paneIds[i])],
            ratio: i / (i + 1),
          };
        }
        setSplitTree(tab.id, node);
      }
      return;
    }

    // Tree exists but new pane was added (not by split) — append it
    const newIds = paneIds.filter(id => !existingIds.includes(id));
    const removedIds = existingIds.filter(id => !paneIds.includes(id));
    let updated: SplitNode | null = tree;
    for (const id of removedIds) {
      updated = updated ? removeNode(updated, id) : null;
    }
    for (const id of newIds) {
      if (updated) {
        updated = { type: 'split', direction: 'horizontal', children: [updated, makeLeaf(id)], ratio: 0.5 };
      } else {
        updated = makeLeaf(id);
      }
    }
    if (updated) setSplitTree(tab.id, updated);
  }, [tab.panes, tab.id]);

  const handleUpdateRatio = useCallback((path: number[], ratio: number) => {
    if (!tree) return;
    const updated = updateRatioAtPath(tree, path, ratio);
    setSplitTree(tab.id, updated);
  }, [tree, tab.id, setSplitTree]);

  // Show shell picker when tab is empty
  if (tab.panes.length === 0) {
    return (
      <div className="tab-empty">
        <div className="tab-shell-picker">
          <div className="tab-shell-picker-title">Select a shell</div>
          {shells.map(shell => (
            <button
              key={shell.id}
              className="tab-shell-picker-btn"
              onClick={() => {
                if (!activeWorkspaceId) return;
                setDefaultShell(shell.id);
                createTerminalInTab(activeWorkspaceId, tab.id, undefined, shell.id);
              }}
            >
              {shell.name}
            </button>
          ))}
          {shells.length === 0 && (
            <span className="tab-shell-picker-hint">No shells detected</span>
          )}
        </div>
      </div>
    );
  }

  if (!tree) return null;

  return (
    <div className="tab-layout">
      <SplitTreeView node={tree} onUpdateRatio={handleUpdateRatio} />
    </div>
  );
}

/** Immutably update the ratio at a given path in the tree. */
function updateRatioAtPath(node: SplitNode, path: number[], ratio: number): SplitNode {
  if (path.length === 0 && node.type === 'split') {
    return { ...node, ratio };
  }
  if (node.type === 'split' && path.length > 0) {
    const [idx, ...rest] = path;
    const children: [SplitNode, SplitNode] = [node.children[0], node.children[1]];
    children[idx] = updateRatioAtPath(children[idx], rest, ratio);
    return { ...node, children };
  }
  return node;
}
