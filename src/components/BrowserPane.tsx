import React, { useEffect, useRef, useCallback, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { listen } from '@tauri-apps/api/event';
import { useStore } from '../store';
import { BrowserMetaBar } from './Sidebar';
import type { BrowserHistoryEntry, BrowserTabInfo, PaneBounds } from '../types';
import './BrowserPane.css';

interface Props {
  /** Unique ID for this browser pane — keys the Rust-side BrowserManager. */
  browserId: string;
  initialUrl?: string;
}

function hostnameOf(url: string): string {
  try { return new URL(url).hostname; } catch { return url; }
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

  const closeBrowserPane = useStore(s => s.closeBrowserPane);
  const setBrowserPaneTitle = useStore(s => s.setBrowserPaneTitle);
  const startPaneDrag = useStore(s => s.startPaneDrag);
  const endPaneDrag = useStore(s => s.endPaneDrag);
  const draggingTerminalId = useStore(s => s.draggingTerminalId);
  const isDragSource = draggingTerminalId === browserId;

  // Keep the sidebar's pane list title in sync with the active tab.
  useEffect(() => {
    const activeTab = tabs.find(t => t.id === activeTabId);
    const title = activeTab?.title || hostnameOf(activeTab?.url ?? urlInput);
    setBrowserPaneTitle(browserId, title);
  }, [browserId, tabs, activeTabId, urlInput, setBrowserPaneTitle]);

  const handleDragStart = useCallback((e: React.PointerEvent) => {
    e.preventDefault();
    startPaneDrag(browserId);
    const handlePointerUp = () => {
      endPaneDrag();
      window.removeEventListener('pointerup', handlePointerUp);
    };
    window.addEventListener('pointerup', handlePointerUp);
  }, [browserId, startPaneDrag, endPaneDrag]);

  // ── Bounds ──────────────────────────────────────────────────────────────────

  const getBounds = useCallback((): PaneBounds | null => {
    const el = containerRef.current;
    if (!el) return null;
    const rect = el.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    // Inset 4 CSS px from both the left and right so the native WebviewWindow
    // doesn't cover the SplitDivider that sits at either edge of the pane
    // (e.g. two browser panes side by side share a divider on the right pane's
    // left edge AND the left pane's right edge).
    const LEFT_INSET = 4;
    const RIGHT_INSET = 4;
    return {
      x: Math.round((rect.left + LEFT_INSET) * dpr),
      y: Math.round(rect.top * dpr),
      width: Math.round((rect.width - LEFT_INSET - RIGHT_INSET) * dpr),
      height: Math.round(rect.height * dpr),
    };
  }, []);

  // Bounds sends are serialized (never more than one in flight) so rapid
  // resize events during a divider drag can't complete out of order — see
  // the same pattern/rationale in TerminalPane.tsx's sendBounds.
  const sendInFlightRef = useRef(false);
  const nextPendingBoundsRef = useRef<PaneBounds | null>(null);

  const sendBounds = useCallback(async (bounds: PaneBounds) => {
    if (sendInFlightRef.current) {
      nextPendingBoundsRef.current = bounds;
      return;
    }
    sendInFlightRef.current = true;
    try {
      await invoke('set_browser_bounds', { browserId, bounds });
    } catch {
      // ignore
    } finally {
      sendInFlightRef.current = false;
    }
    const next = nextPendingBoundsRef.current;
    if (next) {
      nextPendingBoundsRef.current = null;
      sendBounds(next);
    }
  }, [browserId]);

  const rafScheduledRef = useRef(false);
  const openAttemptedRef = useRef(false);

  const reportBounds = useCallback(() => {
    if (rafScheduledRef.current) return;
    rafScheduledRef.current = true;
    requestAnimationFrame(() => {
      rafScheduledRef.current = false;
      const b = getBounds();
      if (b) sendBounds(b);
    });
  }, [getBounds, sendBounds]);

  // ── Lifecycle ───────────────────────────────────────────────────────────────

  useEffect(() => {
    let cancelled = false;
    // Guards against sending a second `open_browser` for this browserId —
    // e.g. React StrictMode / Fast Refresh re-running this effect while the
    // Rust side is still tearing down the previous WebviewWindow for the
    // same label, which otherwise surfaces as "a webview with label ... already
    // exists" since window destruction is not synchronous with the command call.
    if (openAttemptedRef.current) return;
    const interval = setInterval(async () => {
      if (cancelled || browserReady || openAttemptedRef.current) return;
      const bounds = getBounds();
      if (!bounds || bounds.width < 50 || bounds.height < 30) return;
      clearInterval(interval);
      openAttemptedRef.current = true;
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
        openAttemptedRef.current = false;
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

  // Listen for agent-triggered JS eval (fire-and-forget — no return value
  // flows back to the agent, since the OSC channel that triggers this is
  // one-directional; see agent:browser-eval in App.tsx).
  useEffect(() => {
    const unsub = listen<{ js: string }>('ipc:browser-eval', ({ payload }) => {
      const callId = Math.random().toString(36).slice(2);
      invoke('browser_evaluate', { browserId, js: payload.js, callId }).catch(() => {});
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
        <BrowserMetaBar
          browserId={browserId}
          title={hostnameOf(urlInput)}
          onClose={() => closeBrowserPane(browserId)}
          onDragStart={handleDragStart}
        />
        <div style={{ padding: 16, color: '#f7768e', fontSize: 13 }}>Browser failed: {browserError}</div>
      </div>
    );
  }

  return (
    <div className={`browser-pane-wrapper ${isDragSource ? 'browser-pane-drag-source' : ''}`}>
      <BrowserMetaBar
        browserId={browserId}
        title={tabs.find(t => t.id === activeTabId)?.title || hostnameOf(urlInput)}
        onClose={() => closeBrowserPane(browserId)}
        onDragStart={handleDragStart}
      />

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
