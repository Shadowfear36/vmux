import React, { useEffect, useState } from 'react';
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
        </div>
      </div>
    </div>
  );
}
