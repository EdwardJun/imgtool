import { useRef } from 'react';
import type { Rect } from '../types';

interface Props {
  src: string;
  alt: string;
  /** 原图像素尺寸 */
  imageWidth: number;
  imageHeight: number;
  /** 显示尺寸 */
  boxWidth: number;
  rect: Rect;
  /** 宽高比，null 为自由裁剪 */
  ratio: number | null;
  onChange: (rect: Rect) => void;
}

type Handle = 'move' | 'nw' | 'ne' | 'sw' | 'se';

const MIN = 8;

/** 在原图上拖动/缩放裁剪框，坐标以原图像素为单位 */
export function CropEditor({ src, alt, imageWidth, imageHeight, boxWidth, rect, ratio, onChange }: Props) {
  const scale = boxWidth / imageWidth;
  const drag = useRef<{ handle: Handle; startX: number; startY: number; start: Rect } | null>(null);

  const begin = (handle: Handle) => (e: React.PointerEvent) => {
    e.stopPropagation();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    drag.current = { handle, startX: e.clientX, startY: e.clientY, start: rect };
  };

  const onMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    const dx = (e.clientX - d.startX) / scale;
    const dy = (e.clientY - d.startY) / scale;
    const s = d.start;
    if (d.handle === 'move') {
      onChange({
        ...s,
        x: Math.round(Math.min(Math.max(0, s.x + dx), imageWidth - s.width)),
        y: Math.round(Math.min(Math.max(0, s.y + dy), imageHeight - s.height)),
      });
      return;
    }
    // 以对角为锚点缩放
    const right = d.handle.includes('e');
    const bottom = d.handle.includes('s');
    const anchorX = right ? s.x : s.x + s.width;
    const anchorY = bottom ? s.y : s.y + s.height;
    let w = Math.max(MIN, right ? s.width + dx : s.width - dx);
    let h = Math.max(MIN, bottom ? s.height + dy : s.height - dy);
    const maxW = right ? imageWidth - anchorX : anchorX;
    const maxH = bottom ? imageHeight - anchorY : anchorY;
    w = Math.min(w, maxW);
    h = Math.min(h, maxH);
    if (ratio) {
      // 取较小的一边满足比例，保证不越界
      if (w / h > ratio) w = h * ratio;
      else h = w / ratio;
    }
    w = Math.round(w);
    h = Math.round(h);
    onChange({ x: right ? anchorX : anchorX - w, y: bottom ? anchorY : anchorY - h, width: w, height: h });
  };

  const end = () => {
    drag.current = null;
  };

  const style = {
    left: rect.x * scale,
    top: rect.y * scale,
    width: rect.width * scale,
    height: rect.height * scale,
  };

  return (
    <div className="crop-editor checkerboard" onPointerMove={onMove} onPointerUp={end} onPointerCancel={end}>
      <img className="compare-img" src={src} alt={alt} draggable={false} />
      <div className="crop-rect" style={style} onPointerDown={begin('move')}>
        <div className="crop-grid" />
        {(['nw', 'ne', 'sw', 'se'] as const).map((h) => (
          <span key={h} className={`crop-handle ${h}`} onPointerDown={begin(h)} />
        ))}
        <span className="crop-size">
          {rect.width} × {rect.height}
        </span>
      </div>
    </div>
  );
}
