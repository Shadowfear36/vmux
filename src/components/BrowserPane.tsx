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
 * where to position the WebView2 popup window.
 *
 * The 300ms delay before creating the window is critical: React StrictMode
 * runs cleanup between mount cycles, and close_browser in cleanup is a no-op
 * when no window exists yet (timer hasn't fired).
 */
export function BrowserPane({ initialUrl }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [urlInput, setUrlInput] = useState(initialUrl ?? 'https://example.com');
  const [browserReady, setBrowserReady] = useState(false);
  const [browserError, setBrowserError] = useState<string | null>(null);

  const { openBrowser, setBrowserBounds, browserNavigate, browserBack, browserForward, browserReload, setBrowserUrl } = useStore();

  const reportBounds = useCallback(() => {
    const el = containerRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    setBrowserBounds({
      x: Math.round(rect.left * dpr),
      y: Math.round(rect.top * dpr),
      width: Math.round(rect.width * dpr),
      height: Math.round(rect.height * dpr),
    });
  }, [setBrowserBounds]);

  useEffect(() => {
    let cancelled = false;
    let created = false;

    // Poll until the container has valid dimensions, then create the browser.
    // Allotment may not have laid out the pane yet on first render.
    const interval = setInterval(async () => {
      if (cancelled || created) return;
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
      if (bounds.width < 50 || bounds.height < 30) return;
      created = true;
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
    // Periodic bounds refresh — keeps the browser window positioned correctly
    // even when layout shifts don't trigger ResizeObserver.
    const interval = setInterval(reportBounds, 500);
    return () => { observer.disconnect(); clearInterval(interval); };
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

  useEffect(() => {
    const unsub = listen<string>('browser:url-changed', ({ payload }) => {
      setUrlInput(payload);
      setBrowserUrl(payload);
    });
    return () => { unsub.then(f => f()); };
  }, [setBrowserUrl]);

  const handleNavigate = useCallback(() => {
    let url = urlInput.trim();
    if (!url) return;
    if (!/^[a-zA-Z][a-zA-Z\d+\-.]*:/.test(url)) {
      url = 'https://' + url;
      setUrlInput(url);
    }
    browserNavigate(url);
  }, [urlInput, browserNavigate]);

  const handleKeyDown = useCallback((e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') { e.preventDefault(); handleNavigate(); }
    else if (e.key === 'Escape') { (e.target as HTMLInputElement).blur(); }
  }, [handleNavigate]);

  if (browserError) {
    return (
      <div className="browser-pane-wrapper">
        <div style={{ padding: 16, color: '#f7768e', fontSize: 13 }}>
          Browser failed to start: {browserError}
        </div>
      </div>
    );
  }

  return (
    <div className="browser-pane-wrapper">
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
        <button className="browser-go-btn" onClick={() => handleNavigate()}>Go</button>
      </div>
      <div ref={containerRef} className="browser-pane-surface" />
    </div>
  );
}
