import { useMemo, useRef, useState } from 'react';
import type { Bounds, Spec } from '../design/types';
import { flatten, isPlainGroup } from '../design/tree';

interface Props {
  src: string;
  alt: string;
  imageWidth: number;
  imageHeight: number;
  boxWidth: number;
  spec: Spec;
  selected: string | null;
  onSelect: (id: string | null) => void;
  /** 拖动 / 缩放过程中实时更新 */
  onMove: (id: string, bounds: Bounds) => void;
  /** 拖动结束（用于重新取样式） */
  onCommit: (id: string, bounds: Bounds) => void;
  onDraw: (bounds: Bounds) => void;
  onDelete: (id: string) => void;
  /** 只显示到第几层，0 = 全部 */
  depth: number;
  showGroups: boolean;
}

type Handle = 'move' | 'nw' | 'ne' | 'sw' | 'se';
type Drag =
  | { kind: 'edit'; id: string; handle: Handle; startX: number; startY: number; start: Bounds; last: Bounds; moved: boolean }
  | { kind: 'draw'; originX: number; originY: number; current: Bounds };

const MIN = 4;

/**
 * 设计解析的节点叠加层：按类型着色显示每个节点的框和编号。
 * 点框选中（叠在上面的是更深层的节点）；拖动移动、拖四角缩放；在空白处拖动新增框；Delete 删除选中节点。坐标为原图像素。
 */
export function DesignSpecOverlay(p: Props) {
  const { imageWidth, imageHeight, boxWidth, spec, selected } = p;
  const scale = boxWidth / imageWidth;
  const ref = useRef<HTMLDivElement>(null);
  const drag = useRef<Drag | null>(null);
  const [drawing, setDrawing] = useState<Bounds | null>(null);

  const nodes = useMemo(
    () =>
      flatten(spec.root)
        .filter((f) => f.parentId !== null)
        .filter((f) => f.node.id === selected || ((p.depth === 0 || f.depth <= p.depth) && (p.showGroups || !isPlainGroup(f.node)))),
    [spec, selected, p.depth, p.showGroups],
  );

  const toImage = (cx: number, cy: number) => {
    const r = ref.current!.getBoundingClientRect();
    return { x: Math.min(Math.max(0, (cx - r.left) / scale), imageWidth), y: Math.min(Math.max(0, (cy - r.top) / scale), imageHeight) };
  };

  const beginEdit = (id: string, b: Bounds, handle: Handle) => (e: React.PointerEvent) => {
    e.stopPropagation();
    ref.current?.setPointerCapture(e.pointerId);
    ref.current?.focus();
    p.onSelect(id);
    drag.current = { kind: 'edit', id, handle, startX: e.clientX, startY: e.clientY, start: b, last: b, moved: false };
  };

  const beginDraw = (e: React.PointerEvent) => {
    ref.current?.setPointerCapture(e.pointerId);
    ref.current?.focus();
    p.onSelect(null);
    const q = toImage(e.clientX, e.clientY);
    drag.current = { kind: 'draw', originX: q.x, originY: q.y, current: { x: q.x, y: q.y, width: 0, height: 0 } };
  };

  const onPointerMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    if (d.kind === 'draw') {
      const q = toImage(e.clientX, e.clientY);
      d.current = {
        x: Math.round(Math.min(d.originX, q.x)),
        y: Math.round(Math.min(d.originY, q.y)),
        width: Math.round(Math.abs(q.x - d.originX)),
        height: Math.round(Math.abs(q.y - d.originY)),
      };
      setDrawing(d.current);
      return;
    }
    const dx = (e.clientX - d.startX) / scale;
    const dy = (e.clientY - d.startY) / scale;
    if (Math.abs(dx) < 1 && Math.abs(dy) < 1 && !d.moved) return;
    const s = d.start;
    let next: Bounds;
    if (d.handle === 'move') {
      next = {
        ...s,
        x: Math.round(Math.min(Math.max(0, s.x + dx), imageWidth - s.width)),
        y: Math.round(Math.min(Math.max(0, s.y + dy), imageHeight - s.height)),
      };
    } else {
      const right = d.handle.includes('e');
      const bottom = d.handle.includes('s');
      const x0 = Math.max(0, right ? s.x : Math.min(s.x + dx, s.x + s.width - MIN));
      const y0 = Math.max(0, bottom ? s.y : Math.min(s.y + dy, s.y + s.height - MIN));
      const x1 = Math.min(imageWidth, right ? Math.max(s.x + s.width + dx, s.x + MIN) : s.x + s.width);
      const y1 = Math.min(imageHeight, bottom ? Math.max(s.y + s.height + dy, s.y + MIN) : s.y + s.height);
      next = { x: Math.round(x0), y: Math.round(y0), width: Math.round(x1 - x0), height: Math.round(y1 - y0) };
    }
    d.moved = true;
    d.last = next;
    p.onMove(d.id, next);
  };

  const end = () => {
    const d = drag.current;
    drag.current = null;
    if (d?.kind === 'draw') {
      setDrawing(null);
      if (d.current.width >= MIN && d.current.height >= MIN) p.onDraw(d.current);
    } else if (d?.kind === 'edit' && d.moved) {
      p.onCommit(d.id, d.last);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (!selected) return;
    if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault();
      p.onDelete(selected);
    } else if (e.key === 'Escape') {
      p.onSelect(null);
    }
  };

  const style = (b: Bounds) => ({ left: b.x * scale, top: b.y * scale, width: b.width * scale, height: b.height * scale });

  return (
    <div
      ref={ref}
      className="multi-crop design-overlay checkerboard"
      tabIndex={0}
      role="application"
      aria-label="设计稿节点：点击选中，拖动移动，拖四角缩放，在空白处拖动新增框，Delete 删除选中节点"
      onPointerDown={beginDraw}
      onPointerMove={onPointerMove}
      onPointerUp={end}
      onPointerCancel={end}
      onKeyDown={onKeyDown}
    >
      <img className="compare-img" src={p.src} alt={p.alt} draggable={false} />
      {nodes.map(({ node }) => {
        const group = isPlainGroup(node);
        const sel = node.id === selected;
        return (
          <div
            key={node.id}
            className={`ds-rect type-${group ? 'group' : node.type} ${sel ? 'selected' : ''}`}
            style={style(node.bounds)}
            onPointerDown={beginEdit(node.id, node.bounds, 'move')}
            title={`${node.id} ${node.name ?? node.text ?? node.role ?? node.type}`}
          >
            <span className="ds-index">{node.id.replace(/^n/, '')}</span>
            {sel && (['nw', 'ne', 'sw', 'se'] as const).map((h) => <span key={h} className={`crop-handle ${h}`} onPointerDown={beginEdit(node.id, node.bounds, h)} />)}
          </div>
        );
      })}
      {drawing && <div className="ds-rect drawing" style={style(drawing)} />}
    </div>
  );
}
