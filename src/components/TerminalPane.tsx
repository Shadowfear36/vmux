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
  const closeTerminal = useStore(s => s.closeTerminal);
  const startPaneDrag = useStore(s => s.startPaneDrag);
  const endPaneDrag = useStore(s => s.endPaneDrag);
  const draggingTerminalId = useStore(s => s.draggingTerminalId);

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

  // ── Bounds reporting (rAF-throttled) ────────────────────────────────────────
  // During an interactive divider drag, mousemove fires far faster than any
  // fixed debounce window, so a trailing debounce never gets a chance to run
  // until the drag ends — the native HWND would freeze mid-drag and only
  // snap into place on mouseup. Instead we throttle to at most one bounds
  // read+send per animation frame, and always re-send the latest bounds
  // immediately after an in-flight call finishes, so the terminal never
  // drops the final position.

  const initialSentRef = useRef(false);
  const lastBoundsRef = useRef('');
  const rafScheduledRef = useRef(false);
  const sendInFlightRef = useRef(false);
  const nextPendingBoundsRef = useRef<{ x: number; y: number; width: number; height: number } | null>(null);

  const sendBounds = useCallback(async (bounds: { x: number; y: number; width: number; height: number }) => {
    if (sendInFlightRef.current) {
      nextPendingBoundsRef.current = bounds;
      return;
    }
    sendInFlightRef.current = true;
    try {
      await setTerminalBounds(terminalId, bounds);
    } finally {
      sendInFlightRef.current = false;
    }
    const next = nextPendingBoundsRef.current;
    if (next) {
      nextPendingBoundsRef.current = null;
      sendBounds(next);
    }
  }, [terminalId, setTerminalBounds]);

  const reportBounds = useCallback(() => {
    if (rafScheduledRef.current) return;
    rafScheduledRef.current = true;
    requestAnimationFrame(() => {
      rafScheduledRef.current = false;
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

      sendBounds(bounds);
    });
  }, [terminalId, setTerminalBounds, sendBounds]);

  useEffect(() => {
    showTerminal(terminalId);
    return () => hideTerminal(terminalId);
  }, [terminalId]);

  useEffect(() => {
    // Delay first bounds report to let flex layout settle after a split.
    // Concurrent wgpu Phase 2 inits across panes are serialized backend-side
    // (see GPU_INIT_LOCK in renderer.rs), so no client-side stagger is needed.
    const delay = initialSentRef.current ? 0 : 100;
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

  const handleDragStart = useCallback((e: React.PointerEvent) => {
    e.preventDefault();
    startPaneDrag(terminalId);
    const handlePointerUp = () => {
      endPaneDrag();
      window.removeEventListener('pointerup', handlePointerUp);
    };
    window.addEventListener('pointerup', handlePointerUp);
  }, [terminalId, startPaneDrag, endPaneDrag]);

  const isDragSource = draggingTerminalId === terminalId;

  return (
    <div className={`terminal-pane-wrapper ${isFocused ? 'terminal-pane-focused' : ''} ${isDragSource ? 'terminal-pane-drag-source' : ''}`}>
      <TerminalMetaBar
        terminalId={terminalId}
        onClose={() => closeTerminal(terminalId)}
        onDragStart={handleDragStart}
      />
      <div
        ref={containerRef}
        className="terminal-pane-surface"
        onClick={onFocus}
      />
    </div>
  );
}
