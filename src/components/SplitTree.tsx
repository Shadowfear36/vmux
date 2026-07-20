import React, { useRef, useEffect, useState, useCallback } from 'react';
import { TerminalPane } from './TerminalPane';
import { useStore } from '../store';
import './SplitTree.css';

// ─── Split Tree Data Model ───────────────────────────────────────────────────

export type SplitNode =
  | { type: 'leaf'; terminalId: string }
  | { type: 'split'; direction: 'horizontal' | 'vertical'; children: [SplitNode, SplitNode]; ratio: number };

export function makeLeaf(terminalId: string): SplitNode {
  return { type: 'leaf', terminalId };
}

/** Split a leaf node into two, placing the new terminal on the right/bottom. */
export function splitNode(
  tree: SplitNode,
  targetTerminalId: string,
  newTerminalId: string,
  direction: 'horizontal' | 'vertical',
): SplitNode {
  if (tree.type === 'leaf') {
    if (tree.terminalId === targetTerminalId) {
      return {
        type: 'split',
        direction,
        children: [
          { type: 'leaf', terminalId: targetTerminalId },
          { type: 'leaf', terminalId: newTerminalId },
        ],
        ratio: 0.5,
      };
    }
    return tree;
  }
  // Recurse into children
  return {
    ...tree,
    children: [
      splitNode(tree.children[0], targetTerminalId, newTerminalId, direction),
      splitNode(tree.children[1], targetTerminalId, newTerminalId, direction),
    ],
  };
}

/** Remove a terminal from the tree. Returns null if tree becomes empty. */
export function removeNode(tree: SplitNode, terminalId: string): SplitNode | null {
  if (tree.type === 'leaf') {
    return tree.terminalId === terminalId ? null : tree;
  }
  const left = removeNode(tree.children[0], terminalId);
  const right = removeNode(tree.children[1], terminalId);
  if (!left && !right) return null;
  if (!left) return right;
  if (!right) return left;
  return { ...tree, children: [left, right] };
}

/** Get all terminal IDs in the tree. */
export function getTerminalIds(tree: SplitNode): string[] {
  if (tree.type === 'leaf') return [tree.terminalId];
  return [...getTerminalIds(tree.children[0]), ...getTerminalIds(tree.children[1])];
}

// ─── Directional pane navigation ─────────────────────────────────────────────

interface LeafRect { x: number; y: number; w: number; h: number; }

/** Compute each leaf's normalized (0..1) rect within the tree, mirroring the
 *  flex-basis layout SplitTreeView renders (horizontal = row, vertical = column). */
function computeLeafRects(node: SplitNode, rect: LeafRect = { x: 0, y: 0, w: 1, h: 1 }): Record<string, LeafRect> {
  if (node.type === 'leaf') {
    return { [node.terminalId]: rect };
  }
  const { x, y, w, h } = rect;
  if (node.direction === 'horizontal') {
    const w0 = w * node.ratio;
    return {
      ...computeLeafRects(node.children[0], { x, y, w: w0, h }),
      ...computeLeafRects(node.children[1], { x: x + w0, y, w: w - w0, h }),
    };
  }
  const h0 = h * node.ratio;
  return {
    ...computeLeafRects(node.children[0], { x, y, w, h: h0 }),
    ...computeLeafRects(node.children[1], { x, y: y + h0, w, h: h - h0 }),
  };
}

/** Find the nearest leaf in a given direction from the focused terminal,
 *  using pane-center geometry (tmux/i3-style: prefer close in the travel
 *  axis, penalize being off-center on the perpendicular axis). */
export function findPaneInDirection(
  tree: SplitNode,
  focusedTerminalId: string,
  direction: 'up' | 'down' | 'left' | 'right',
): string | null {
  const rects = computeLeafRects(tree);
  const focused = rects[focusedTerminalId];
  if (!focused) return null;
  const fcx = focused.x + focused.w / 2;
  const fcy = focused.y + focused.h / 2;
  const EPS = 1e-6;

  let best: string | null = null;
  let bestScore = Infinity;

  for (const [id, r] of Object.entries(rects)) {
    if (id === focusedTerminalId) continue;
    const cx = r.x + r.w / 2;
    const cy = r.y + r.h / 2;

    let inDirection = false;
    let primary = 0;
    let perpendicular = 0;
    switch (direction) {
      case 'left':  inDirection = cx < fcx - EPS; primary = fcx - cx; perpendicular = Math.abs(cy - fcy); break;
      case 'right': inDirection = cx > fcx + EPS; primary = cx - fcx; perpendicular = Math.abs(cy - fcy); break;
      case 'up':    inDirection = cy < fcy - EPS; primary = fcy - cy; perpendicular = Math.abs(cx - fcx); break;
      case 'down':  inDirection = cy > fcy + EPS; primary = cy - fcy; perpendicular = Math.abs(cx - fcx); break;
    }
    if (!inDirection) continue;

    const score = primary + perpendicular * 2;
    if (score < bestScore) {
      bestScore = score;
      best = id;
    }
  }

  return best;
}

// ─── Split Tree Renderer ─────────────────────────────────────────────────────

interface SplitTreeViewProps {
  node: SplitNode;
  onUpdateRatio?: (path: number[], ratio: number) => void;
  path?: number[];
}

export function SplitTreeView({ node, onUpdateRatio, path = [] }: SplitTreeViewProps) {
  const { focusedTerminalId, focusTerminal } = useStore();

  if (node.type === 'leaf') {
    return (
      <div className="split-leaf">
        <TerminalPane
          terminalId={node.terminalId}
          isFocused={focusedTerminalId === node.terminalId}
          onFocus={() => focusTerminal(node.terminalId)}
        />
      </div>
    );
  }

  const isHorizontal = node.direction === 'horizontal';

  // Use flex-basis so the flex container properly allocates space.
  // "0 0 <basis>" means no grow, no shrink, exactly this size.
  const pct1 = `calc(${node.ratio * 100}% - 2px)`;
  const pct2 = `calc(${(1 - node.ratio) * 100}% - 2px)`;

  return (
    <div className={`split-container ${isHorizontal ? 'split-h' : 'split-v'}`}>
      <div className="split-child" style={{ flex: `0 0 ${pct1}` }}>
        <SplitTreeView node={node.children[0]} onUpdateRatio={onUpdateRatio} path={[...path, 0]} />
      </div>
      <SplitDivider
        direction={node.direction}
        ratio={node.ratio}
        onRatioChange={(newRatio) => {
          if (!onUpdateRatio) return;
          onUpdateRatio(path, newRatio);
        }}
      />
      <div className="split-child" style={{ flex: `0 0 ${pct2}` }}>
        <SplitTreeView node={node.children[1]} onUpdateRatio={onUpdateRatio} path={[...path, 1]} />
      </div>
    </div>
  );
}

// ─── Resizable Divider ───────────────────────────────────────────────────────

function SplitDivider({ direction, ratio, onRatioChange }: {
  direction: 'horizontal' | 'vertical';
  ratio: number;
  onRatioChange: (newRatio: number) => void;
}) {
  const ref = useRef<HTMLDivElement>(null);

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    const isH = direction === 'horizontal';
    const startPos = isH ? e.clientX : e.clientY;
    const startRatio = ratio;
    const parent = ref.current?.parentElement;
    if (!parent) return;
    const rect = parent.getBoundingClientRect();
    const parentSize = isH ? rect.width : rect.height;
    if (parentSize < 1) return;

    const handleMouseMove = (ev: MouseEvent) => {
      const pos = isH ? ev.clientX : ev.clientY;
      const delta = (pos - startPos) / parentSize;
      const newRatio = Math.max(0.1, Math.min(0.9, startRatio + delta));
      onRatioChange(newRatio);
    };

    const handleMouseUp = () => {
      window.removeEventListener('mousemove', handleMouseMove);
      window.removeEventListener('mouseup', handleMouseUp);
    };

    window.addEventListener('mousemove', handleMouseMove);
    window.addEventListener('mouseup', handleMouseUp);
  }, [direction, ratio, onRatioChange]);

  return (
    <div
      ref={ref}
      className={`split-divider ${direction === 'horizontal' ? 'split-divider-h' : 'split-divider-v'}`}
      onMouseDown={handleMouseDown}
    />
  );
}
