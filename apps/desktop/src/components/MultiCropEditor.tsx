import { useRef, useState } from 'react';
import type { Rect, Region } from '../types';

interface Props {
  src: string;
  alt: string;
  imageWidth: number;
  imageHeight: number;
  /** 显示宽度 */
  boxWidth: number;
  regions: Region[];
  selected: number | null;
  onSelect: (i: number | null) => void;
  /** 框被拖动/缩放/新增/删除 */
  onChange: (regions: Region[]) => void;
}

type Handle = 'move' | 'nw' | 'ne' | 'sw' | 'se';
type Drag =
  | { kind: 'edit'; index: number; handle: Handle; startX: number; startY: number; start: Rect }
  | { kind: 'draw'; originX: number; originY: number; current: Rect };

const MIN = 6;

/**
 * 智能裁剪的多框编辑器：
 * 点框选中并拖动；拖选中框的四角调整大小；在空白处拖动新增框；Delete/Backspace 删除选中框。
 * 坐标均为原图像素。
 */
export function MultiCropEditor({ src, alt, imageWidth, imageHeight, boxWidth, regions, selected, onSelect, onChange }: Props) {
  const scale = boxWidth / imageWidth;
  const ref = useRef<HTMLDivElement>(null);
  const drag = useRef<Drag | null>(null);
  const [drawing, setDrawing] = useState<Rect | null>(null);

  const toImage = (clientX: number, clientY: number) => {
    const r = ref.current!.getBoundingClientRect();
    return {
      x: Math.min(Math.max(0, (clientX - r.left) / scale), imageWidth),
      y: Math.min(Math.max(0, (clientY - r.top) / scale), imageHeight),
    };
  };

  const beginEdit = (index: number, handle: Handle) => (e: React.PointerEvent) => {
    e.stopPropagation();
    ref.current?.setPointerCapture(e.pointerId);
    ref.current?.focus();
    onSelect(index);
    const { x, y, width, height } = regions[index];
    drag.current = { kind: 'edit', index, handle, startX: e.clientX, startY: e.clientY, start: { x, y, width, height } };
  };

  const beginDraw = (e: React.PointerEvent) => {
    ref.current?.setPointerCapture(e.pointerId);
    ref.current?.focus();
    onSelect(null);
    const p = toImage(e.clientX, e.clientY);
    drag.current = { kind: 'draw', originX: p.x, originY: p.y, current: { x: p.x, y: p.y, width: 0, height: 0 } };
  };

  const onMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    if (d.kind === 'draw') {
      const p = toImage(e.clientX, e.clientY);
      d.current = {
        x: Math.round(Math.min(d.originX, p.x)),
        y: Math.round(Math.min(d.originY, p.y)),
        width: Math.round(Math.abs(p.x - d.originX)),
        height: Math.round(Math.abs(p.y - d.originY)),
      };
      setDrawing(d.current);
      return;
    }
    const dx = (e.clientX - d.startX) / scale;
    const dy = (e.clientY - d.startY) / scale;
    const s = d.start;
    let next: Rect;
    if (d.handle === 'move') {
      next = {
        ...s,
        x: Math.round(Math.min(Math.max(0, s.x + dx), imageWidth - s.width)),
        y: Math.round(Math.min(Math.max(0, s.y + dy), imageHeight - s.height)),
      };
    } else {
      const right = d.handle.includes('e');
      const bottom = d.handle.includes('s');
      let x0 = right ? s.x : Math.min(s.x + dx, s.x + s.width - MIN);
      let y0 = bottom ? s.y : Math.min(s.y + dy, s.y + s.height - MIN);
      let x1 = right ? Math.max(s.x + s.width + dx, s.x + MIN) : s.x + s.width;
      let y1 = bottom ? Math.max(s.y + s.height + dy, s.y + MIN) : s.y + s.height;
      x0 = Math.max(0, x0);
      y0 = Math.max(0, y0);
      x1 = Math.min(imageWidth, x1);
      y1 = Math.min(imageHeight, y1);
      next = { x: Math.round(x0), y: Math.round(y0), width: Math.round(x1 - x0), height: Math.round(y1 - y0) };
    }
    onChange(regions.map((r, i) => (i === d.index ? { ...r, ...next } : r)));
  };

  const end = () => {
    const d = drag.current;
    drag.current = null;
    if (d?.kind === 'draw') {
      setDrawing(null);
      if (d.current.width >= MIN && d.current.height >= MIN) {
        onChange([...regions, { ...d.current, kind: 'icon' }]);
        onSelect(regions.length);
      }
    }
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (selected === null) return;
    if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault();
      onChange(regions.filter((_, i) => i !== selected));
      onSelect(null);
    } else if (e.key === 'Escape') {
      onSelect(null);
    }
  };

  const style = (r: Rect) => ({ left: r.x * scale, top: r.y * scale, width: r.width * scale, height: r.height * scale });

  return (
    <div
      ref={ref}
      className="multi-crop checkerboard"
      tabIndex={0}
      role="application"
      aria-label="智能裁剪框编辑：在空白处拖动新增框，Delete 删除选中框"
      onPointerDown={beginDraw}
      onPointerMove={onMove}
      onPointerUp={end}
      onPointerCancel={end}
      onKeyDown={onKeyDown}
    >
      <img className="compare-img" src={src} alt={alt} draggable={false} />
      {regions.map((r, i) => (
        <div key={i} className={`mc-rect ${i === selected ? 'selected' : ''}`} style={style(r)} onPointerDown={beginEdit(i, 'move')}>
          <span className="mc-index">{i + 1}</span>
          {i === selected &&
            (['nw', 'ne', 'sw', 'se'] as const).map((h) => <span key={h} className={`crop-handle ${h}`} onPointerDown={beginEdit(i, h)} />)}
        </div>
      ))}
      {drawing && <div className="mc-rect drawing" style={style(drawing)} />}
    </div>
  );
}
