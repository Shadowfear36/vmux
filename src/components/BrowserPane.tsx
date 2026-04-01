import React, { useEffect, useRef, useCallback, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { listen } from '@tauri-apps/api/event';
import { useStore } from '../store';
import type { BrowserHistoryEntry } from '../types';
import './BrowserPane.css';

interface Props {
  initialUrl?: string;
}

export function BrowserPane({ initialUrl }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [urlInput, setUrlInput] = useState(initialUrl ?? 'https://example.com');
  const [browserReady, setBrowserReady] = useState(false);
  const [browserError, setBrowserError] = useState<string | null>(null);
  const [showHistory, setShowHistory] = useState(false);
  const [history, setHistory] = useState<BrowserHistoryEntry[]>([]);

  const {
    openBrowser, openBrowserTab, closeBrowserTab, switchBrowserTab,
    setBrowserBounds, browserNavigate, browserBack, browserForward,
    browserReload, browserOpenDevtools, setBrowserUrl,
    browserTabs, activeBrowserTabId, showBrowser,
  } = useStore();

  // ── Bounds ──────────────────────────────────────────────────────────────────

  const getBounds = useCallback(() => {
    const el = containerRef.current;
    if (!el) return null;
    const rect = el.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    return {
      x: Math.round(rect.left * dpr),
      y: Math.round(rect.top * dpr),
      width: Math.round(rect.width * dpr),
      height: Math.round(rect.height * dpr),
    };
  }, []);

  const reportBounds = useCallback(() => {
    const b = getBounds();
    if (b) setBrowserBounds(b);
  }, [getBounds, setBrowserBounds]);

  // ── Lifecycle ───────────────────────────────────────────────────────────────

  useEffect(() => {
    let cancelled = false;
    const interval = setInterval(async () => {
      if (cancelled || browserReady) return;
      const bounds = getBounds();
      if (!bounds || bounds.width < 50 || bounds.height < 30) return;
      clearInterval(interval);
      try {
        await openBrowser(bounds, initialUrl);
        if (!cancelled) setBrowserReady(true);
      } catch (err) {
        console.error('[vmux] openBrowser failed:', err);
        if (!cancelled) setBrowserError(String(err));
      }
    }, 150);

    return () => {
      cancelled = true;
      clearInterval(interval);
      invoke('close_browser').catch(() => {});
    };
  }, []);

  useEffect(() => {
    if (!browserReady) return;
    reportBounds();
    const observer = new ResizeObserver(reportBounds);
    if (containerRef.current) observer.observe(containerRef.current);
    const poll = setInterval(reportBounds, 500);
    return () => { observer.disconnect(); clearInterval(poll); };
  }, [reportBounds, browserReady]);

  // When showBrowser toggles on, immediately re-report bounds
  // (Allotment keeps us mounted but resizes the pane)
  useEffect(() => {
    if (showBrowser && browserReady) {
      // Small delay for Allotment to finish layout
      const t = setTimeout(reportBounds, 50);
      return () => clearTimeout(t);
    }
  }, [showBrowser, browserReady, reportBounds]);

  useEffect(() => {
    window.addEventListener('resize', reportBounds);
    let unlisten: (() => void) | null = null;
    getCurrentWindow().onMoved(() => reportBounds()).then(fn => { unlisten = fn; });
    return () => { window.removeEventListener('resize', reportBounds); unlisten?.(); };
  }, [reportBounds]);

  useEffect(() => {
    const unsub = listen<string>('browser:url-changed', ({ payload }) => {
      setUrlInput(payload);
      setBrowserUrl(payload);
    });
    return () => { unsub.then(f => f()); };
  }, [setBrowserUrl]);

  // ── Handlers ────────────────────────────────────────────────────────────────

  const handleNavigate = useCallback(() => {
    let url = urlInput.trim();
    if (!url) return;
    if (!/^[a-zA-Z][a-zA-Z\d+\-.]*:/.test(url)) url = 'https://' + url;
    setUrlInput(url);
    browserNavigate(url);
  }, [urlInput, browserNavigate]);

  const handleKeyDown = useCallback((e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') { e.preventDefault(); handleNavigate(); }
    else if (e.key === 'Escape') { (e.target as HTMLInputElement).blur(); }
  }, [handleNavigate]);

  const handleNewTab = useCallback(async () => {
    const bounds = getBounds();
    if (!bounds) return;
    await openBrowserTab(bounds, 'https://example.com');
  }, [getBounds, openBrowserTab]);

  const handleCloseTab = useCallback(async (tabId: string) => {
    const bounds = getBounds() ?? { x: 0, y: 0, width: 800, height: 600 };
    await closeBrowserTab(tabId, bounds);
  }, [getBounds, closeBrowserTab]);

  const handleSwitchTab = useCallback(async (tabId: string) => {
    const bounds = getBounds() ?? { x: 0, y: 0, width: 800, height: 600 };
    await switchBrowserTab(tabId, bounds);
    const tab = useStore.getState().browserTabs.find(t => t.id === tabId);
    if (tab) setUrlInput(tab.url);
  }, [getBounds, switchBrowserTab]);

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
      {browserTabs.length > 0 && (
        <div className="browser-tabbar">
          {browserTabs.map(tab => (
            <div
              key={tab.id}
              className={`browser-tab ${tab.id === activeBrowserTabId ? 'browser-tab-active' : ''}`}
              onClick={() => handleSwitchTab(tab.id)}
            >
              <span className="browser-tab-url">
                {tab.title || (() => { try { return new URL(tab.url).hostname; } catch { return tab.url; } })()}
              </span>
              {browserTabs.length > 1 && (
                <button className="browser-tab-close" onClick={e => { e.stopPropagation(); handleCloseTab(tab.id); }}>x</button>
              )}
            </div>
          ))}
          <button className="browser-tab-new" onClick={handleNewTab} title="New tab">+</button>
        </div>
      )}

      {/* URL toolbar */}
      <div className="browser-toolbar">
        <button className="browser-nav-btn" onClick={browserBack} title="Back">&#x2190;</button>
        <button className="browser-nav-btn" onClick={browserForward} title="Forward">&#x2192;</button>
        <button className="browser-nav-btn" onClick={browserReload} title="Reload">&#x21BA;</button>
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
        <button className="browser-nav-btn browser-devtools-btn" onClick={browserOpenDevtools} title="DevTools">Dev</button>
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
              browserNavigate(h.url);
              setUrlInput(h.url);
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

      {/* Transparent surface — WebView2 renders here */}
      <div ref={containerRef} className="browser-pane-surface" />
    </div>
  );
}
