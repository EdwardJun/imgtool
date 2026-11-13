import { useCallback, useRef, useState } from 'react';

interface Props {
  before: string;
  after?: string;
  alt: string;
}

/** 左原图、右处理结果，拖动中线对比 */
export function CompareView({ before, after, alt }: Props) {
  const [pos, setPos] = useState(50);
  const ref = useRef<HTMLDivElement>(null);

  const update = useCallback((clientX: number) => {
    const rect = ref.current?.getBoundingClientRect();
    if (!rect) return;
    setPos(Math.min(100, Math.max(0, ((clientX - rect.left) / rect.width) * 100)));
  }, []);

  const onPointerDown = (e: React.PointerEvent) => {
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    update(e.clientX);
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowLeft') setPos((p) => Math.max(0, p - 5));
    if (e.key === 'ArrowRight') setPos((p) => Math.min(100, p + 5));
  };

  return (
    <div
      ref={ref}
      className="compare checkerboard"
      onPointerDown={onPointerDown}
      onPointerMove={(e) => e.buttons === 1 && update(e.clientX)}
    >
      <img className="compare-img" src={before} alt={`${alt} 原图`} draggable={false} />
      {after && (
        <img
          className="compare-img"
          src={after}
          alt={`${alt} 处理后`}
          draggable={false}
          style={{ clipPath: `inset(0 0 0 ${pos}%)` }}
        />
      )}
      {after && (
        <>
          <span className="compare-tag left">原图</span>
          <span className="compare-tag right">处理后</span>
          <div
            className="compare-handle"
            style={{ left: `${pos}%` }}
            role="slider"
            tabIndex={0}
            aria-label="对比分割线"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round(pos)}
            onKeyDown={onKeyDown}
          >
            <span className="compare-knob">‹ ›</span>
          </div>
        </>
      )}
    </div>
  );
}
