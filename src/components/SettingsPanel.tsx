import React, { useEffect, useState } from 'react';
import { ask } from '@tauri-apps/plugin-dialog';
import { useStore } from '../store';
import type { Settings } from '../types';
import './SettingsPanel.css';

interface Props {
  onClose: () => void;
}

const THEMES = [
  { id: 'tokyo_night', label: 'Tokyo Night' },
  { id: 'catppuccin_mocha', label: 'Catppuccin Mocha' },
];

export function SettingsPanel({ onClose }: Props) {
  const { settings, loadSettings, updateSettings, shells } = useStore();
  const hasVmuxBrowserSkill = useStore(s => s.hasVmuxBrowserSkill);
  const installVmuxBrowserSkill = useStore(s => s.installVmuxBrowserSkill);
  const hasVmuxContextSkill = useStore(s => s.hasVmuxContextSkill);
  const installVmuxContextSkill = useStore(s => s.installVmuxContextSkill);
  const [form, setForm] = useState<Settings | null>(settings);
  const [saving, setSaving] = useState(false);
  const [prefixKeyError, setPrefixKeyError] = useState<string | null>(null);

  useEffect(() => {
    if (!settings) loadSettings();
  }, []);

  useEffect(() => {
    if (settings) setForm(settings);
  }, [settings]);

  const handleSave = async () => {
    if (!form) return;
    const key = form.prefix_key.trim().toLowerCase();
    if (!/^[a-z]$/.test(key)) {
      setPrefixKeyError('Prefix key must be a single letter (a-z).');
      return;
    }
    setPrefixKeyError(null);
    setSaving(true);
    try {
      await updateSettings({ ...form, prefix_key: key });
    } finally {
      setSaving(false);
    }
  };

  if (!form) {
    return (
      <div className="settings-overlay" onClick={onClose}>
        <div className="settings-panel" onClick={e => e.stopPropagation()}>
          <div className="settings-header">
            <span>Settings</span>
            <button className="settings-close" onClick={onClose}>x</button>
          </div>
          <div className="settings-body">Loading...</div>
        </div>
      </div>
    );
  }

  return (
    <div className="settings-overlay" onClick={onClose}>
      <div className="settings-panel" onClick={e => e.stopPropagation()}>
        <div className="settings-header">
          <span>Settings</span>
          <button className="settings-close" onClick={onClose}>x</button>
        </div>
        <div className="settings-body">
          <label className="settings-field">
            <span className="settings-label">Theme</span>
            <select
              className="settings-select"
              value={form.theme_name}
              onChange={e => setForm({ ...form, theme_name: e.target.value })}
            >
              {THEMES.map(t => <option key={t.id} value={t.id}>{t.label}</option>)}
            </select>
          </label>

          <label className="settings-field">
            <span className="settings-label">Default shell</span>
            <select
              className="settings-select"
              value={form.default_shell_id ?? ''}
              onChange={e => setForm({ ...form, default_shell_id: e.target.value || null })}
            >
              <option value="">Auto (first detected)</option>
              {shells.map(s => <option key={s.id} value={s.id}>{s.name}</option>)}
            </select>
          </label>

          <label className="settings-field">
            <span className="settings-label">Font size</span>
            <input
              className="settings-input"
              type="number"
              min={8}
              max={32}
              step={1}
              value={form.font_size}
              onChange={e => setForm({ ...form, font_size: Number(e.target.value) || form.font_size })}
            />
          </label>

          <label className="settings-field">
            <span className="settings-label">Prefix key (Ctrl-&lt;key&gt;)</span>
            <input
              className="settings-input settings-input-small"
              type="text"
              maxLength={1}
              value={form.prefix_key}
              onChange={e => setForm({ ...form, prefix_key: e.target.value })}
            />
          </label>
          {prefixKeyError && <div className="settings-error">{prefixKeyError}</div>}

          <label className="settings-field">
            <span className="settings-label">Open file command</span>
            <input
              className="settings-input"
              type="text"
              placeholder="vim %f  (default)"
              value={form.open_file_command ?? ''}
              onChange={e => setForm({ ...form, open_file_command: e.target.value || null })}
            />
          </label>
          <div className="settings-hint settings-hint-small">
            Command run when opening a file from the file tree. Use <code>%f</code> for the path.
            Leave empty to use the default (<code>vim</code>).
          </div>

          <div className="settings-hint">
            Theme and font size apply immediately to every open terminal. Prefix key
            changes apply to new keystrokes right away — existing terminals don't
            need to be reopened.
          </div>

          <div className="settings-actions">
            <button className="settings-save-btn" onClick={handleSave} disabled={saving}>
              {saving ? 'Saving...' : 'Save'}
            </button>
          </div>

          <SkillInstallRow
            title="Claude Code: browser-control skill"
            description={<>Lets Claude Code (running in any vmux terminal, in any project) open, navigate,
              and close the browser pane. Installs a skill file to your global{' '}
              <code>~/.claude/skills/</code> — not tied to this project.</>}
            consentMessage="This writes ~/.claude/skills/vmux-browser/SKILL.md, a short doc teaching Claude Code
              (and any other agent that reads Claude Code skills) how to control the vmux browser pane
              from any terminal, in any project. It only adds/overwrites that one file. Install now?"
            consentTitle="Install vmux browser skill?"
            hasFn={hasVmuxBrowserSkill}
            installFn={installVmuxBrowserSkill}
          />

          <SkillInstallRow
            title="Claude Code: context-search skill"
            description={<>Lets Claude Code search past conversation history and notes vmux has imported
              (across every project), and pull a full past conversation once it finds the relevant one.
              Installs a skill file to your global <code>~/.claude/skills/</code> — not tied to this project.</>}
            consentMessage="This writes ~/.claude/skills/vmux-context/SKILL.md, a short doc teaching Claude Code
              (and any other agent that reads Claude Code skills) how to search vmux's conversation history
              and notes from any terminal, in any project. It only adds/overwrites that one file. Install now?"
            consentTitle="Install vmux context-search skill?"
            hasFn={hasVmuxContextSkill}
            installFn={installVmuxContextSkill}
          />

          <DaemonSessionsSection />
        </div>
      </div>
    </div>
  );
}

/**
 * Experimental (VMUX_DAEMON_TERMINALS): lists vmuxd-owned sessions and any
 * orphaned processes left over from a previous crash, with Kill buttons for
 * both. See docs/session-reattach-design.md §17.
 */
function DaemonSessionsSection() {
  const daemonRunning = useStore(s => s.daemonRunning);
  const daemonSessions = useStore(s => s.daemonSessions);
  const daemonOrphans = useStore(s => s.daemonOrphans);
  const loadDaemonSessions = useStore(s => s.loadDaemonSessions);
  const killDaemonSession = useStore(s => s.killDaemonSession);
  const killDaemonOrphan = useStore(s => s.killDaemonOrphan);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    loadDaemonSessions().finally(() => setLoaded(true));
  }, []);

  return (
    <div className="settings-field" style={{ marginTop: 20, borderTop: '1px solid #292e42', paddingTop: 16 }}>
      <span className="settings-label">Daemon sessions (experimental)</span>
      <div className="settings-hint settings-hint-small">
        Terminals started with <code>VMUX_DAEMON_TERMINALS</code> set survive closing vmux — the shell/agent process
        keeps running in a background <code>vmuxd</code> process and gets reattached on next launch.
      </div>

      {!loaded ? (
        <div className="settings-hint settings-hint-small" style={{ marginTop: 8 }}>Checking...</div>
      ) : !daemonRunning ? (
        <div className="settings-hint settings-hint-small" style={{ marginTop: 8 }}>Daemon not running.</div>
      ) : (
        <>
          <div style={{ marginTop: 8 }}>
            {daemonSessions.length === 0 ? (
              <div className="settings-hint settings-hint-small">No active sessions.</div>
            ) : (
              daemonSessions.map(s => (
                <div key={s.session_id} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', fontSize: 12, padding: '4px 0' }}>
                  <span>{s.label} (pid {s.pid ?? '?'}) — {s.attached_clients} attached</span>
                  <button className="settings-save-btn" style={{ padding: '2px 8px' }} onClick={() => killDaemonSession(s.session_id)}>
                    Kill
                  </button>
                </div>
              ))
            )}
          </div>

          {daemonOrphans.length > 0 && (
            <div style={{ marginTop: 12 }}>
              <div className="settings-hint settings-hint-small">
                Found from a previous crash — unreachable but still running:
              </div>
              {daemonOrphans.map(o => (
                <div key={o.pid} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', fontSize: 12, padding: '4px 0' }}>
                  <span>{o.label} (pid {o.pid})</span>
                  <button className="settings-save-btn" style={{ padding: '2px 8px' }} onClick={() => killDaemonOrphan(o.pid)}>
                    Kill
                  </button>
                </div>
              ))}
            </div>
          )}

          <div className="settings-actions" style={{ marginTop: 8 }}>
            <button className="settings-save-btn" onClick={() => loadDaemonSessions()}>Refresh</button>
          </div>
        </>
      )}
    </div>
  );
}

interface SkillInstallRowProps {
  title: string;
  description: React.ReactNode;
  consentMessage: string;
  consentTitle: string;
  hasFn: () => Promise<boolean>;
  installFn: () => Promise<void>;
}

/** A single "install this Claude Code skill" row: status check, consent dialog, install/reinstall button. */
function SkillInstallRow({ title, description, consentMessage, consentTitle, hasFn, installFn }: SkillInstallRowProps) {
  const [installed, setInstalled] = useState<boolean | null>(null);
  const [installing, setInstalling] = useState(false);

  useEffect(() => {
    hasFn().then(setInstalled).catch(() => setInstalled(false));
  }, [hasFn]);

  const handleInstall = async () => {
    const consent = await ask(consentMessage, { title: consentTitle, kind: 'info' });
    if (!consent) return;
    setInstalling(true);
    try {
      await installFn();
      setInstalled(true);
    } finally {
      setInstalling(false);
    }
  };

  return (
    <div className="settings-field" style={{ marginTop: 20, borderTop: '1px solid #292e42', paddingTop: 16 }}>
      <span className="settings-label">{title}</span>
      <div className="settings-hint settings-hint-small">{description}</div>
      <div className="settings-actions" style={{ marginTop: 8 }}>
        <button
          className="settings-save-btn"
          onClick={handleInstall}
          disabled={installing || installed === null}
        >
          {installing ? 'Installing...' : installed ? 'Reinstall / update skill' : 'Install skill'}
        </button>
        {installed && <span style={{ marginLeft: 10, fontSize: 12, color: '#9ece6a' }}>Installed</span>}
      </div>
    </div>
  );
}
