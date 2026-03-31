import React, { useEffect, useRef, useCallback, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { listen } from '@tauri-apps/api/event';
import { useStore } from '../store';
import './BrowserPane.css';

interface Props {
  initialUrl?: string;
}

/**
 * BrowserPane renders a transparent placeholder div and tells the Rust backend
 * where to position the WebView2 child pane. Mirrors the TerminalPane pattern.
 *
 * IMPORTANT: cleanup must NOT call closeBrowser() (which sets showBrowser=false)
 * because React StrictMode runs cleanup between mount cycles, which would
 * toggle the browser off immediately after toggling it on.
 */
export function BrowserPane({ initialUrl }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [urlInput, setUrlInput] = useState(initialUrl ?? 'https://example.com');
  const [isEditing, setIsEditing] = useState(false);
  const [browserReady, setBrowserReady] = useState(false);

  const { openBrowser, setBrowserBounds, browserNavigate, browserBack, browserForward, browserReload, setBrowserUrl } = useStore();

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

  const [browserError, setBrowserError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    // Delay to let layout settle
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
      // Only close the backend webview — do NOT set showBrowser=false
      // (that's the caller's job via toggleBrowser/closeBrowser)
      invoke('close_browser').catch(() => {});
    };
  }, []);

  useEffect(() => {
    if (!browserReady) return;
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
    const unsub = listen<string>('browser:url-changed', ({ payload }) => {
      setUrlInput(payload);
      setBrowserUrl(payload);
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
      {/* URL toolbar */}
      <div className="browser-toolbar">
        <button className="browser-nav-btn" onClick={browserBack} title="Back">←</button>
        <button className="browser-nav-btn" onClick={browserForward} title="Forward">→</button>
        <button className="browser-nav-btn" onClick={browserReload} title="Reload">↺</button>
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
        <span className="browser-cdp-hint" title="Playwright can connect to localhost:9222">CDP:9222</span>
      </div>

      {/* Transparent surface — WebView2 renders behind/over this */}
      <div ref={containerRef} className="browser-pane-surface" />
    </div>
  );
}
