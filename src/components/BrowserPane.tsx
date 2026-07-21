import React, { useEffect, useRef, useCallback, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { listen } from '@tauri-apps/api/event';
import type { BrowserHistoryEntry, BrowserTabInfo, PaneBounds } from '../types';
import './BrowserPane.css';

interface Props {
  /** Unique ID for this browser pane — keys the Rust-side BrowserManager. */
  browserId: string;
  initialUrl?: string;
}

export function BrowserPane({ browserId, initialUrl }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [urlInput, setUrlInput] = useState(initialUrl ?? 'https://example.com');
  const [browserReady, setBrowserReady] = useState(false);
  const [browserError, setBrowserError] = useState<string | null>(null);
  const [showHistory, setShowHistory] = useState(false);
  const [history, setHistory] = useState<BrowserHistoryEntry[]>([]);
  const [tabs, setTabs] = useState<BrowserTabInfo[]>([]);
  const [activeTabId, setActiveTabId] = useState<string | null>(null);

  // ── Bounds ──────────────────────────────────────────────────────────────────

  const getBounds = useCallback((): PaneBounds | null => {
    const el = containerRef.current;
    if (!el) return null;
    const rect = el.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    // Inset 4 CSS px from the left so the native WebviewWindow doesn't
    // cover the SplitDivider that sits at the pane's left edge.
    const LEFT_INSET = 4;
    return {
      x: Math.round((rect.left + LEFT_INSET) * dpr),
      y: Math.round(rect.top * dpr),
      width: Math.round((rect.width - LEFT_INSET) * dpr),
      height: Math.round(rect.height * dpr),
    };
  }, []);

  const reportBounds = useCallback(() => {
    const b = getBounds();
    if (b) invoke('set_browser_bounds', { browserId, bounds: b }).catch(() => {});
  }, [getBounds, browserId]);

  // ── Lifecycle ───────────────────────────────────────────────────────────────

  useEffect(() => {
    let cancelled = false;
    const interval = setInterval(async () => {
      if (cancelled || browserReady) return;
      const bounds = getBounds();
      if (!bounds || bounds.width < 50 || bounds.height < 30) return;
      clearInterval(interval);
      try {
        const tabId = await invoke<string>('open_browser', {
          browserId,
          bounds,
          url: initialUrl ?? null,
        });
        if (!cancelled) {
          setTabs([{ id: tabId, url: initialUrl ?? 'https://example.com', title: '' }]);
          setActiveTabId(tabId);
          setBrowserReady(true);
        }
      } catch (err) {
        console.error('[vmux] open_browser failed:', err);
        if (!cancelled) setBrowserError(String(err));
      }
    }, 150);

    return () => {
      cancelled = true;
      clearInterval(interval);
      invoke('close_browser', { browserId }).catch(() => {});
    };
  }, [browserId]);

  useEffect(() => {
    if (!browserReady) return;
    reportBounds();
    const observer = new ResizeObserver(reportBounds);
    if (containerRef.current) observer.observe(containerRef.current);
    const poll = setInterval(reportBounds, 500);
    return () => { observer.disconnect(); clearInterval(poll); };
  }, [reportBounds, browserReady]);

  useEffect(() => {
    window.addEventListener('resize', reportBounds);
    let unlisten: (() => void) | null = null;
    getCurrentWindow().onMoved(() => reportBounds()).then(fn => { unlisten = fn; });
    return () => { window.removeEventListener('resize', reportBounds); unlisten?.(); };
  }, [reportBounds]);

  // Route URL change events to this pane only
  useEffect(() => {
    const unsub = listen<{ browserId: string; url: string }>('browser:url-changed', ({ payload }) => {
      if (payload.browserId !== browserId) return;
      setUrlInput(payload.url);
      setTabs(prev => prev.map(t => t.id === activeTabId ? { ...t, url: payload.url } : t));
    });
    return () => { unsub.then(f => f()); };
  }, [browserId, activeTabId]);

  // Listen for IPC-triggered navigation
  useEffect(() => {
    const unsub = listen<{ url: string }>('ipc:browser-navigate', ({ payload }) => {
      handleNavigateToUrl(payload.url);
    });
    return () => { unsub.then(f => f()); };
  }, [browserId]);

  // ── Handlers ────────────────────────────────────────────────────────────────

  const handleNavigateToUrl = useCallback((url: string) => {
    invoke('browser_navigate', { browserId, url }).catch(() => {});
    setUrlInput(url);
  }, [browserId]);

  const handleNavigate = useCallback(() => {
    let url = urlInput.trim();
    if (!url) return;
    if (!/^[a-zA-Z][a-zA-Z\d+\-.]*:/.test(url)) url = 'https://' + url;
    setUrlInput(url);
    invoke('browser_navigate', { browserId, url }).catch(() => {});
  }, [urlInput, browserId]);

  const handleKeyDown = useCallback((e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') { e.preventDefault(); handleNavigate(); }
    else if (e.key === 'Escape') { (e.target as HTMLInputElement).blur(); }
  }, [handleNavigate]);

  const handleNewTab = useCallback(async () => {
    const bounds = getBounds();
    if (!bounds) return;
    const tabId = await invoke<string>('open_browser_tab', {
      browserId, bounds, url: 'https://example.com',
    });
    setTabs(prev => [...prev, { id: tabId, url: 'https://example.com', title: '' }]);
    setActiveTabId(tabId);
    setUrlInput('https://example.com');
  }, [getBounds, browserId]);

  const handleCloseTab = useCallback(async (tabId: string) => {
    const bounds = getBounds() ?? { x: 0, y: 0, width: 800, height: 600 };
    const remaining = await invoke<BrowserTabInfo[]>('close_browser_tab', { browserId, tabId, bounds });
    setTabs(remaining);
    if (remaining.length > 0) {
      const next = remaining[remaining.length - 1];
      setActiveTabId(next.id);
      setUrlInput(next.url);
    }
  }, [getBounds, browserId]);

  const handleSwitchTab = useCallback(async (tabId: string) => {
    const bounds = getBounds() ?? { x: 0, y: 0, width: 800, height: 600 };
    await invoke('switch_browser_tab', { browserId, tabId, bounds });
    setActiveTabId(tabId);
    const tab = tabs.find(t => t.id === tabId);
    if (tab) setUrlInput(tab.url);
  }, [getBounds, browserId, tabs]);

  const toggleHistory = useCallback(async () => {
    if (!showHistory) {
      try {
        const h = await invoke<BrowserHistoryEntry[]>('browser_history', { limit: 50 });
        setHistory(h);
      } catch { /* ignore */ }
    }
    setShowHistory(s => !s);
  }, [showHistory]);

  if (browserError) {
    return (
      <div className="browser-pane-wrapper">
        <div style={{ padding: 16, color: '#f7768e', fontSize: 13 }}>Browser failed: {browserError}</div>
      </div>
    );
  }

  return (
    <div className="browser-pane-wrapper">
      {/* Tab bar */}
      {tabs.length > 0 && (
        <div className="browser-tabbar">
          {tabs.map(tab => (
            <div
              key={tab.id}
              className={`browser-tab ${tab.id === activeTabId ? 'browser-tab-active' : ''}`}
              onClick={() => handleSwitchTab(tab.id)}
            >
              <span className="browser-tab-url">
                {tab.title || (() => { try { return new URL(tab.url).hostname; } catch { return tab.url; } })()}
              </span>
              {tabs.length > 1 && (
                <button className="browser-tab-close" onClick={e => { e.stopPropagation(); handleCloseTab(tab.id); }}>x</button>
              )}
            </div>
          ))}
          <button className="browser-tab-new" onClick={handleNewTab} title="New tab">+</button>
        </div>
      )}

      {/* URL toolbar */}
      <div className="browser-toolbar">
        <button className="browser-nav-btn" onClick={() => invoke('browser_back', { browserId })} title="Back">&#x2190;</button>
        <button className="browser-nav-btn" onClick={() => invoke('browser_forward', { browserId })} title="Forward">&#x2192;</button>
        <button className="browser-nav-btn" onClick={() => invoke('browser_reload', { browserId })} title="Reload">&#x21BA;</button>
        <input
          className="browser-url-input"
          type="text"
          value={urlInput}
          onChange={e => setUrlInput(e.target.value)}
          onKeyDown={handleKeyDown}
          spellCheck={false}
        />
        <button className="browser-go-btn" onClick={handleNavigate}>Go</button>
        <button className="browser-nav-btn" onClick={toggleHistory} title="History">&#x1F553;</button>
        <button className="browser-nav-btn browser-devtools-btn" onClick={() => invoke('browser_open_devtools', { browserId })} title="DevTools">Dev</button>
      </div>

      {/* History dropdown */}
      {showHistory && (
        <div className="browser-history-dropdown">
          <div className="browser-history-header">
            <span>History</span>
            <button className="browser-history-clear" onClick={async () => {
              await invoke('clear_browser_history');
              setHistory([]);
            }}>Clear</button>
          </div>
          {history.length === 0 && <div className="browser-history-empty">No history yet</div>}
          {history.map(h => (
            <div key={h.id} className="browser-history-item" onClick={() => {
              handleNavigateToUrl(h.url);
              setShowHistory(false);
            }}>
              <span className="browser-history-url">{h.title ?? h.url}</span>
              <span className="browser-history-time">
                {new Date(h.visited_at * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
              </span>
            </div>
          ))}
        </div>
      )}

      {/* Transparent surface — WebView2 native window renders here */}
      <div ref={containerRef} className="browser-pane-surface" />
    </div>
  );
}
