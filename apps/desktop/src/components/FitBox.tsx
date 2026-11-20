import { useLayoutEffect, useRef, useState, type ReactNode } from 'react';

interface Props {
  /** 内容的原始宽高，用于等比适配 */
  width: number;
  height: number;
  /** 相对"适应窗口"的缩放倍数；不传时只做适应（不超过原图大小） */
  zoom?: number;
  /** 按住 Ctrl / ⌘ 滚动（或触控板双指捏合）时调用，参数为新的缩放倍数 */
  onZoomChange?: (zoom: number) => void;
  /** "适应窗口"时的显示比例（显示像素 / 原图像素，≤ 1），用于显示实际百分比 */
  onFitChange?: (fit: number) => void;
  /** 抓手模式：在画面上直接拖动即可平移（否则按住空格或鼠标中键拖动） */
  panMode?: boolean;
  children: (size: { width: number; height: number }) => ReactNode;
}

export const MIN_ZOOM = 0.25;
export const MAX_ZOOM = 16;
export const clampZoom = (z: number) => Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, z));

/** 在可用区域内等比缩放出一个与图片同比例的盒子；传 zoom 时可放大缩小、拖动平移，超出部分滚动查看 */
export function FitBox({ width, height, zoom, onZoomChange, onFitChange, panMode, children }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [avail, setAvail] = useState({ width: 0, height: 0 });
  /** 缩放后要保持在光标下的内容点（0-1 比例）与光标在容器里的位置 */
  const anchor = useRef<{ fx: number; fy: number; cx: number; cy: number } | null>(null);

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

  const fit = width && height ? Math.min(avail.width / width, avail.height / height, 1) : 0;
  const scale = fit * (zoom ?? 1);
  useLayoutEffect(() => {
    if (fit > 0) onFitChange?.(fit);
  }, [fit, onFitChange]);
  const size = { width: Math.max(1, Math.floor(width * scale)), height: Math.max(1, Math.floor(height * scale)) };

  // 缩放后把光标下的那一点放回光标下面
  useLayoutEffect(() => {
    const el = ref.current;
    const a = anchor.current;
    if (!el || !a) return;
    anchor.current = null;
    const inner = el.firstElementChild as HTMLElement | null;
    if (!inner) return;
    el.scrollLeft = inner.offsetLeft + a.fx * size.width - a.cx;
    el.scrollTop = inner.offsetTop + a.fy * size.height - a.cy;
  }, [size.width, size.height]);

  // React 的 onWheel 是 passive 的，无法阻止浏览器自带的页面缩放，这里手动注册
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || !onZoomChange || zoom === undefined) return;
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey) return;
      e.preventDefault();
      const inner = el.firstElementChild as HTMLElement | null;
      if (!inner) return;
      const r = el.getBoundingClientRect();
      const cx = e.clientX - r.left;
      const cy = e.clientY - r.top;
      anchor.current = {
        fx: (el.scrollLeft + cx - inner.offsetLeft) / inner.offsetWidth,
        fy: (el.scrollTop + cy - inner.offsetTop) / inner.offsetHeight,
        cx,
        cy,
      };
      onZoomChange(clampZoom(zoom * Math.exp(-e.deltaY * 0.01)));
    };
    el.addEventListener('wheel', onWheel, { passive: false });
    return () => el.removeEventListener('wheel', onWheel);
  }, [zoom, onZoomChange]);

  // 拖动画面：抓手模式下直接拖、按住空格拖、或按住鼠标中键拖。在捕获阶段拦截，避免触发框选
  const [spaceDown, setSpaceDown] = useState(false);
  const [panning, setPanning] = useState(false);
  useLayoutEffect(() => {
    if (zoom === undefined) return;
    const typing = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      return !!t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable);
    };
    const down = (e: KeyboardEvent) => {
      if (e.code === 'Space' && !typing(e)) {
        e.preventDefault();
        setSpaceDown(true);
      }
    };
    const up = (e: KeyboardEvent) => e.code === 'Space' && setSpaceDown(false);
    const blur = () => setSpaceDown(false);
    window.addEventListener('keydown', down);
    window.addEventListener('keyup', up);
    window.addEventListener('blur', blur);
    return () => {
      window.removeEventListener('keydown', down);
      window.removeEventListener('keyup', up);
      window.removeEventListener('blur', blur);
    };
  }, [zoom === undefined]);
  const panActive = zoom !== undefined && (!!panMode || spaceDown);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || zoom === undefined) return;
    let drag: { id: number; x: number; y: number; sl: number; st: number } | null = null;
    const onDown = (e: PointerEvent) => {
      if (!(panActive || e.button === 1)) return;
      e.preventDefault();
      e.stopPropagation();
      el.setPointerCapture(e.pointerId);
      drag = { id: e.pointerId, x: e.clientX, y: e.clientY, sl: el.scrollLeft, st: el.scrollTop };
      setPanning(true);
    };
    const onMove = (e: PointerEvent) => {
      if (!drag || e.pointerId !== drag.id) return;
      e.preventDefault();
      e.stopPropagation();
      el.scrollLeft = drag.sl - (e.clientX - drag.x);
      el.scrollTop = drag.st - (e.clientY - drag.y);
    };
    const onUp = (e: PointerEvent) => {
      if (!drag || e.pointerId !== drag.id) return;
      e.stopPropagation();
      drag = null;
      setPanning(false);
    };
    el.addEventListener('pointerdown', onDown, true);
    el.addEventListener('pointermove', onMove, true);
    el.addEventListener('pointerup', onUp, true);
    el.addEventListener('pointercancel', onUp, true);
    return () => {
      el.removeEventListener('pointerdown', onDown, true);
      el.removeEventListener('pointermove', onMove, true);
      el.removeEventListener('pointerup', onUp, true);
      el.removeEventListener('pointercancel', onUp, true);
    };
  }, [panActive, zoom === undefined]);

  return (
    <div ref={ref} className={`fitbox ${zoom !== undefined ? 'zoomable' : ''} ${panActive ? 'pan-mode' : ''} ${panning ? 'panning' : ''}`}>
      {fit > 0 && (
        <div className="fitbox-inner" style={size}>
          {children(size)}
        </div>
      )}
    </div>
  );
}
