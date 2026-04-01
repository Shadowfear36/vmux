import React from 'react';
import './KeyboardHelp.css';

type ShortcutItem = { section: string } | { key: string; desc: string };

const SHORTCUTS: ShortcutItem[] = [
  { section: 'Panes' },
  { key: 'Ctrl-A c', desc: 'Split horizontal (default shell)' },
  { key: 'Ctrl-A -', desc: 'Split vertical (default shell)' },
  { key: 'Ctrl-A d', desc: 'Close focused pane' },

  { section: 'Shell Picker (chord)' },
  { key: 'Ctrl-A s 1-9', desc: 'Open shell #N horizontal' },
  { key: 'Ctrl-A s - 1-9', desc: 'Open shell #N vertical' },

  { section: 'Agent Picker (chord)' },
  { key: 'Ctrl-A a 1-9', desc: 'Open agent #N horizontal' },
  { key: 'Ctrl-A a - 1-9', desc: 'Open agent #N vertical' },

  { section: 'Tabs' },
  { key: 'Ctrl-A t', desc: 'New tab' },
  { key: 'Ctrl-A n', desc: 'Next tab' },
  { key: 'Ctrl-A p', desc: 'Previous tab' },

  { section: 'Git Worktrees (chord)' },
  { key: 'Ctrl-A w n', desc: 'New worktree (prompts branch)' },
  { key: 'Ctrl-A w +', desc: 'New workspace' },

  { section: 'Panels' },
  { key: 'Ctrl-A b', desc: 'Toggle browser' },
  { key: 'Ctrl-A x', desc: 'Toggle context panel' },
  { key: 'Ctrl-A f', desc: 'Toggle file tree' },
  { key: 'Ctrl-A g', desc: 'Toggle git diff panel' },

  { section: 'Other' },
  { key: 'Ctrl-A Ctrl-A', desc: 'Send literal Ctrl-A' },
  { key: 'Ctrl-A ?', desc: 'Toggle this help' },
];

interface Props {
  onClose: () => void;
}

export function KeyboardHelp({ onClose }: Props) {
  return (
    <div className="kb-help-overlay" onClick={onClose}>
      <div className="kb-help-panel" onClick={e => e.stopPropagation()}>
        <div className="kb-help-header">
          <span>Keyboard Shortcuts</span>
          <button className="kb-help-close" onClick={onClose}>x</button>
        </div>
        <div className="kb-help-body">
          {SHORTCUTS.map((item, i) =>
            'section' in item ? (
              <div key={i} className="kb-help-section">{item.section}</div>
            ) : (
              <div key={i} className="kb-help-row">
                <kbd className="kb-help-key">{item.key}</kbd>
                <span className="kb-help-desc">{item.desc}</span>
              </div>
            )
          )}
        </div>
        <div className="kb-help-footer">
          Prefix: <kbd>Ctrl-A</kbd> then key. Chords: prefix → category → action.
        </div>
      </div>
    </div>
  );
}
