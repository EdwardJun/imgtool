import { useLayoutEffect, useRef, useState, type ReactNode } from 'react';

interface Props {
  /** 内容的原始宽高，用于等比适配 */
  width: number;
  height: number;
  children: (size: { width: number; height: number }) => ReactNode;
}

/** 在可用区域内等比缩放出一个与图片同比例的盒子 */
export function FitBox({ width, height, children }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [avail, setAvail] = useState({ width: 0, height: 0 });

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) => {
      const { width: w, height: h } = entry.contentRect;
      setAvail({ width: w, height: h });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const scale = width && height ? Math.min(avail.width / width, avail.height / height, 1) : 0;
  const size = { width: Math.floor(width * scale), height: Math.floor(height * scale) };

  return (
    <div ref={ref} className="fitbox">
      {scale > 0 && (
        <div className="fitbox-inner" style={size}>
          {children(size)}
        </div>
      )}
    </div>
  );
}
