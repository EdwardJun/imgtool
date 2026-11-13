import { Button, InputNumber, Slider } from 'antd';
import type { AspectPreset, CropSettings, ImageItem, Rect } from '../types';

interface Props {
  value: CropSettings;
  current?: ImageItem;
  rect?: Rect;
  onChange: (v: CropSettings) => void;
  onRectChange: (r: Rect) => void;
  onApplyAll: () => void;
}

const ASPECTS: { label: string; value: AspectPreset }[] = [
  { label: '自由', value: 'free' },
  { label: '原比例', value: 'original' },
  { label: '1:1', value: '1:1' },
  { label: '4:3', value: '4:3' },
  { label: '3:4', value: '3:4' },
  { label: '16:9', value: '16:9' },
  { label: '9:16', value: '9:16' },
];

export function CropPanel({ value, current, rect, onChange, onRectChange, onApplyAll }: Props) {
  const set = (patch: Partial<CropSettings>) => onChange({ ...value, ...patch });
  const W = current?.info.width ?? 0;
  const H = current?.info.height ?? 0;

  const setRect = (patch: Partial<Rect>) => {
    if (!rect) return;
    const next = { ...rect, ...patch };
    next.x = Math.min(Math.max(0, next.x), W - 1);
    next.y = Math.min(Math.max(0, next.y), H - 1);
    next.width = Math.min(Math.max(1, next.width), W - next.x);
    next.height = Math.min(Math.max(1, next.height), H - next.y);
    onRectChange(next);
  };

  return (
    <div className="panel-section">
      <div className="panel-title">裁剪比例</div>
      <div className="chip-grid">
        {ASPECTS.map((a) => (
          <button key={a.value} className={`chip ${value.aspect === a.value ? 'active' : ''}`} aria-pressed={value.aspect === a.value} onClick={() => set({ aspect: a.value })}>
            {a.label}
          </button>
        ))}
      </div>

      <div className="panel-title">裁剪区域（原图像素）</div>
      {rect ? (
        <div className="rect-grid">
          <InputNumber aria-label="X" prefix="X" min={0} max={W - 1} value={rect.x} onChange={(v) => setRect({ x: v ?? 0 })} />
          <InputNumber aria-label="Y" prefix="Y" min={0} max={H - 1} value={rect.y} onChange={(v) => setRect({ y: v ?? 0 })} />
          <InputNumber aria-label="宽" prefix="宽" min={1} max={W} value={rect.width} disabled={value.aspect !== 'free'} onChange={(v) => setRect({ width: v ?? 1 })} />
          <InputNumber aria-label="高" prefix="高" min={1} max={H} value={rect.height} disabled={value.aspect !== 'free'} onChange={(v) => setRect({ height: v ?? 1 })} />
        </div>
      ) : (
        <div className="panel-hint block">请先添加图片</div>
      )}
      <div className="panel-hint block">在左侧拖动裁剪框，拖四角调整大小</div>
      <Button block size="small" onClick={onApplyAll} disabled={!current}>
        按当前比例重置所有图片的裁剪框
      </Button>

      <div className="panel-title">裁剪后缩放到（可选）</div>
      <div className="size-row">
        <InputNumber aria-label="输出宽" min={1} max={20000} value={value.outWidth} onChange={(v) => set({ outWidth: v })} suffix="宽" style={{ flex: 1 }} />
        <span className="lock-btn static">×</span>
        <InputNumber aria-label="输出高" min={1} max={20000} value={value.outHeight} onChange={(v) => set({ outHeight: v })} suffix="高" style={{ flex: 1 }} />
      </div>

      <div className="panel-title">输出质量</div>
      <div className="slider-row">
        <Slider min={1} max={100} value={value.quality} onChange={(q) => set({ quality: q })} style={{ flex: 1 }} />
        <InputNumber aria-label="质量" min={1} max={100} value={value.quality} onChange={(q) => set({ quality: q ?? 90 })} style={{ width: 64 }} />
      </div>
    </div>
  );
}
