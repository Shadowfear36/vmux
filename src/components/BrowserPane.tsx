import React, { useEffect, useRef, useCallback, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { listen } from '@tauri-apps/api/event';
import { useStore } from '../store';
import './BrowserPane.css';

interface Props {
  initialUrl?: string;
}

export function BrowserPane({ initialUrl }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [urlInput, setUrlInput] = useState(initialUrl ?? 'https://example.com');
  const [isEditing, setIsEditing] = useState(false);
  const [browserReady, setBrowserReady] = useState(false);
  const [browserError, setBrowserError] = useState<string | null>(null);

  const {
    openBrowser, openBrowserTab, closeBrowserTab, switchBrowserTab,
    setBrowserBounds, browserNavigate, browserBack, browserForward,
    browserReload, browserOpenDevtools, setBrowserUrl,
    browserTabs, activeBrowserTabId,
  } = useStore();

  // ── Bounds reporting ────────────────────────────────────────────────────────

  const reportBounds = useCallback(() => {
    const el = containerRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    setBrowserBounds({
      x: Math.round(rect.left   * dpr),
      y: Math.round(rect.top    * dpr),
      width:  Math.round(rect.width  * dpr),
      height: Math.round(rect.height * dpr),
    });
  }, [setBrowserBounds]);

  // ── Lifecycle ───────────────────────────────────────────────────────────────

  useEffect(() => {
    let cancelled = false;

    const timer = setTimeout(async () => {
      if (cancelled) return;
      const el = containerRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      const dpr = window.devicePixelRatio || 1;
      const bounds = {
        x: Math.round(rect.left   * dpr),
        y: Math.round(rect.top    * dpr),
        width:  Math.round(rect.width  * dpr),
        height: Math.round(rect.height * dpr),
      };
      if (bounds.width < 50 || bounds.height < 30) return;
      try {
        await openBrowser(bounds, initialUrl);
        if (!cancelled) setBrowserReady(true);
      } catch (err) {
        console.error('[vmux] openBrowser failed:', err);
        if (!cancelled) setBrowserError(String(err));
      }
    }, 300);

    return () => {
      cancelled = true;
      clearTimeout(timer);
      invoke('close_browser').catch(() => {});
    };
  }, []);

  useEffect(() => {
    if (!browserReady) return;
    // Report bounds immediately and on any resize
    reportBounds();
    const observer = new ResizeObserver(reportBounds);
    if (containerRef.current) observer.observe(containerRef.current);
    return () => observer.disconnect();
  }, [reportBounds, browserReady]);

  useEffect(() => {
    window.addEventListener('resize', reportBounds);
    let unlisten: (() => void) | null = null;
    getCurrentWindow().onMoved(() => reportBounds()).then(fn => { unlisten = fn; });
    return () => {
      window.removeEventListener('resize', reportBounds);
      unlisten?.();
    };
  }, [reportBounds]);

  // ── URL bar sync ───────────────────────────────────────────────────────────

  useEffect(() => {
    const unsub = listen<{ tabId: string; url: string } | string>('browser:url-changed', ({ payload }) => {
      // Handle both new format {tabId, url} and legacy string format
      const url = typeof payload === 'string' ? payload : payload.url;
      const tabId = typeof payload === 'string' ? null : payload.tabId;
      setUrlInput(url);
      setBrowserUrl(url);
      // Update tab URL in store
      if (tabId) {
        useStore.setState(s => ({
          browserTabs: s.browserTabs.map(t =>
            t.id === tabId ? { ...t, url } : t
          ),
        }));
      }
    });
    return () => { unsub.then(f => f()); };
  }, [setBrowserUrl]);

  // ── URL bar handlers ───────────────────────────────────────────────────────

  const handleNavigate = useCallback(() => {
    let url = urlInput.trim();
    if (!url) return;
    if (!/^[a-zA-Z][a-zA-Z\d+\-.]*:/.test(url)) {
      url = 'https://' + url;
      setUrlInput(url);
    }
    browserNavigate(url);
    setIsEditing(false);
  }, [urlInput, browserNavigate]);

  const handleKeyDown = useCallback((e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      handleNavigate();
    } else if (e.key === 'Escape') {
      setIsEditing(false);
    }
  }, [handleNavigate]);

  const handleNewTab = useCallback(async () => {
    const el = containerRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    const bounds = {
      x: Math.round(rect.left * dpr),
      y: Math.round(rect.top * dpr),
      width: Math.round(rect.width * dpr),
      height: Math.round(rect.height * dpr),
    };
    await openBrowserTab(bounds, 'https://example.com');
  }, [openBrowserTab]);

  if (browserError) {
    return (
      <div className="browser-pane-wrapper">
        <div style={{ padding: 16, color: '#f7768e', fontSize: 13 }}>
          Browser failed to start: {browserError}
          <br /><br />
          <button onClick={() => setBrowserError(null)}
            style={{ padding: '4px 12px', cursor: 'pointer' }}>
            Retry
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="browser-pane-wrapper">
      {/* Browser tab bar */}
      {browserTabs.length > 0 && (
        <div className="browser-tabbar">
          {browserTabs.map(tab => (
            <div
              key={tab.id}
              className={`browser-tab ${tab.id === activeBrowserTabId ? 'browser-tab-active' : ''}`}
              onClick={() => {
                switchBrowserTab(tab.id);
                setUrlInput(tab.url);
              }}
            >
              <span className="browser-tab-url">
                {(() => {
                  try { return new URL(tab.url).hostname || tab.url; }
                  catch { return tab.url; }
                })()}
              </span>
              <button
                className="browser-tab-close"
                onClick={e => { e.stopPropagation(); closeBrowserTab(tab.id); }}
              >x</button>
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
          onFocus={() => setIsEditing(true)}
          onBlur={() => setIsEditing(false)}
          onKeyDown={handleKeyDown}
          spellCheck={false}
        />
        <button className="browser-go-btn" onClick={handleNavigate}>Go</button>
        <button className="browser-nav-btn browser-devtools-btn" onClick={browserOpenDevtools} title="DevTools (F12)">
          DevTools
        </button>
      </div>

      {/* Transparent surface -- WebView2 renders behind/over this */}
      <div ref={containerRef} className="browser-pane-surface" />
    </div>
  );
}
