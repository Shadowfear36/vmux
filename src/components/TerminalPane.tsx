import React, { useEffect, useRef, useCallback } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { listen } from '@tauri-apps/api/event';
import { useStore } from '../store';
import { TerminalMetaBar } from './Sidebar';
import './TerminalPane.css';

interface Props {
  terminalId: string;
  isFocused: boolean;
  onFocus: () => void;
}

/**
 * TerminalPane is a transparent placeholder that tells the Rust backend
 * where to position the native Win32 child window.
 *
 * ALL keyboard input is handled natively in the Win32 WndProc (window.rs).
 * This component only manages bounds reporting and focus tracking.
 */
export function TerminalPane({ terminalId, isFocused, onFocus }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const setTerminalBounds = useStore(s => s.setTerminalBounds);
  const showTerminal = useStore(s => s.showTerminal);
  const hideTerminal = useStore(s => s.hideTerminal);

  // ── Focus tracking ─────────────────────────────────────────────────────────
  // The HWND takes keyboard focus natively. These events just keep the
  // frontend's focusedTerminalId in sync for UI highlights.

  useEffect(() => {
    const unsub = listen<{ terminalId: string }>('terminal:clicked', ({ payload }) => {
      if (payload.terminalId === terminalId) onFocus();
    });
    return () => { unsub.then(f => f()); };
  }, [terminalId, onFocus]);

  useEffect(() => {
    const unsub = listen<{ terminalId: string }>('terminal:ready', ({ payload }) => {
      if (payload.terminalId === terminalId) onFocus();
    });
    return () => { unsub.then(f => f()); };
  }, [terminalId, onFocus]);

  // ── Bounds reporting (debounced) ───────────────────────────────────────────

  const boundsTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const initialSentRef = useRef(false);
  const lastBoundsRef = useRef('');
  const pendingRef = useRef(false);

  const reportBounds = useCallback(() => {
    const el = containerRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    const bounds = {
      x: Math.round(rect.left  * dpr),
      y: Math.round(rect.top   * dpr),
      width:  Math.round(rect.width  * dpr),
      height: Math.round(rect.height * dpr),
    };

    if (bounds.width < 50 || bounds.height < 30) return;

    // Skip if bounds haven't actually changed
    const key = `${bounds.x},${bounds.y},${bounds.width},${bounds.height}`;
    if (key === lastBoundsRef.current) return;
    lastBoundsRef.current = key;

    // First call: immediate (triggers Phase 2 init).
    if (!initialSentRef.current) {
      initialSentRef.current = true;
      setTerminalBounds(terminalId, bounds);
      return;
    }

    // Subsequent: debounce at 60ms and serialize (skip if a call is in-flight)
    if (boundsTimerRef.current) clearTimeout(boundsTimerRef.current);
    boundsTimerRef.current = setTimeout(async () => {
      if (pendingRef.current) return;
      pendingRef.current = true;
      try {
        await setTerminalBounds(terminalId, bounds);
      } finally {
        pendingRef.current = false;
      }
    }, 60);
  }, [terminalId, setTerminalBounds]);

  useEffect(() => {
    showTerminal(terminalId);
    return () => hideTerminal(terminalId);
  }, [terminalId]);

  useEffect(() => {
    // Delay first bounds report to let flex layout settle after a split.
    // Stagger by a random offset to avoid concurrent wgpu Phase 2 inits
    // which can crash when multiple surfaces init simultaneously.
    const delay = initialSentRef.current ? 0 : 100 + Math.random() * 200;
    const initDelay = setTimeout(() => {
      reportBounds();
    }, delay);
    const observer = new ResizeObserver(reportBounds);
    if (containerRef.current) observer.observe(containerRef.current);
    const poll = setInterval(reportBounds, 300);
    return () => { clearTimeout(initDelay); observer.disconnect(); clearInterval(poll); };
  }, [reportBounds]);

  useEffect(() => {
    window.addEventListener('resize', reportBounds);
    let unlisten: (() => void) | null = null;
    getCurrentWindow().onMoved(() => reportBounds()).then(fn => { unlisten = fn; });
    return () => {
      window.removeEventListener('resize', reportBounds);
      unlisten?.();
    };
  }, [reportBounds]);

  return (
    <div className={`terminal-pane-wrapper ${isFocused ? 'terminal-pane-focused' : ''}`}>
      <TerminalMetaBar terminalId={terminalId} />
      <div
        ref={containerRef}
        className="terminal-pane-surface"
        onClick={onFocus}
      />
    </div>
  );
}
