import React, { useEffect, useRef } from 'react';
import { Allotment } from 'allotment';
import 'allotment/dist/style.css';
import { useStore } from '../store';
import { TerminalPane } from './TerminalPane';
import type { Tab, Pane } from '../types';
import './TabView.css';

interface Props {
  tab: Tab;
}

export function TabView({ tab }: Props) {
  const { createTerminalInTab, focusedTerminalId, focusTerminal, activeWorkspaceId } = useStore();
  const creatingRef = useRef(false);

  // Auto-create a terminal if the tab is empty (guard prevents StrictMode double-fire)
  useEffect(() => {
    if (tab.panes.length === 0 && activeWorkspaceId && !creatingRef.current) {
      creatingRef.current = true;
      setTimeout(() => {
        createTerminalInTab(activeWorkspaceId, tab.id)
          .catch(e => console.error('[vmux] createTerminalInTab failed:', e))
          .finally(() => { creatingRef.current = false; });
      }, 0);
    }
  }, [tab.id]);

  if (tab.panes.length === 0) {
    return (
      <div className="tab-empty">
        <span className="tab-empty-hint">Opening terminal...</span>
      </div>
    );
  }

  const terminalPanes = tab.panes.filter(p => p.kind.type === 'terminal');

  // Always use a stable Allotment structure — mapping panes with keys.
  // This prevents unmount/remount of existing panes when adding new ones.
  return (
    <div className="tab-layout">
      <Allotment>
        {terminalPanes.map(pane => {
          const tid = getTerminalId(pane);
          return (
            <Allotment.Pane key={pane.id} minSize={100}>
              <PaneView
                pane={pane}
                isFocused={focusedTerminalId === tid}
                onFocus={() => tid && focusTerminal(tid)}
              />
            </Allotment.Pane>
          );
        })}
      </Allotment>
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
