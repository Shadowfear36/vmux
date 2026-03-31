import React, { useRef, useState } from 'react';
import { useStore } from '../store';
import { TerminalPane } from './TerminalPane';
import type { Tab, Pane } from '../types';
import './TabView.css';

interface Props {
  tab: Tab;
}

export function TabView({ tab }: Props) {
  const {
    createTerminalInTab, focusedTerminalId, focusTerminal,
    activeWorkspaceId, reorderPanes,
    shells, setDefaultShell,
  } = useStore();
  const [dragOverId, setDragOverId] = useState<string | null>(null);
  const dragSrcId = useRef<string | null>(null);

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

  const terminalPanes = tab.panes.filter(p => p.kind.type === 'terminal');
  const isVertical = tab.direction === 'vertical';

  const handleDragStart = (paneId: string) => { dragSrcId.current = paneId; };
  const handleDragOver = (e: React.DragEvent, paneId: string) => {
    e.preventDefault();
    if (dragSrcId.current && dragSrcId.current !== paneId) setDragOverId(paneId);
  };
  const handleDrop = (targetPaneId: string) => {
    setDragOverId(null);
    const srcId = dragSrcId.current;
    dragSrcId.current = null;
    if (!srcId || srcId === targetPaneId || !activeWorkspaceId) return;
    const ids = terminalPanes.map(p => p.id);
    const srcIdx = ids.indexOf(srcId);
    const tgtIdx = ids.indexOf(targetPaneId);
    if (srcIdx === -1 || tgtIdx === -1) return;
    ids.splice(srcIdx, 1);
    ids.splice(tgtIdx, 0, srcId);
    reorderPanes(activeWorkspaceId, tab.id, ids);
  };
  const handleDragEnd = () => { dragSrcId.current = null; setDragOverId(null); };

  return (
    <div className={`tab-layout ${isVertical ? 'tab-layout-stacked' : 'tab-layout-side'}`}>
      {terminalPanes.map(pane => {
        const tid = getTerminalId(pane);
        return (
          <div
            key={pane.id}
            className={`tab-pane ${dragOverId === pane.id ? 'tab-pane-drag-over' : ''}`}
            draggable
            onDragStart={() => handleDragStart(pane.id)}
            onDragOver={(e) => handleDragOver(e, pane.id)}
            onDragLeave={() => setDragOverId(null)}
            onDrop={() => handleDrop(pane.id)}
            onDragEnd={handleDragEnd}
          >
            <PaneView
              pane={pane}
              isFocused={focusedTerminalId === tid}
              onFocus={() => tid && focusTerminal(tid)}
            />
          </div>
        );
      })}
    </div>
  );
}

function PaneView({ pane, isFocused, onFocus }: { pane: Pane; isFocused: boolean; onFocus: () => void }) {
  if (pane.kind.type === 'terminal') {
    return <TerminalPane terminalId={pane.kind.terminal_id} isFocused={isFocused} onFocus={onFocus} />;
  }
  return <div />;
}

function getTerminalId(pane: Pane): string | null {
  return pane.kind.type === 'terminal' ? pane.kind.terminal_id : null;
}
