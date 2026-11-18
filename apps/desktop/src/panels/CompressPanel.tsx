import { InputNumber, Radio, Segmented, Select, Slider, Space, Tooltip } from 'antd';
import { LinkOutlined, DisconnectOutlined } from '@ant-design/icons';
import type { CompressSettings, ImageItem, ResizeMode } from '../types';
import { FORMAT_LABEL, OUTPUT_FORMATS } from '../types';

interface Props {
  value: CompressSettings;
  current?: ImageItem;
  onChange: (v: CompressSettings) => void;
}

const RESIZE_OPTIONS: { label: string; value: ResizeMode }[] = [
  { label: '原尺寸', value: 'original' },
  { label: '百分比', value: 'percent' },
  { label: '最大宽高', value: 'max' },
  { label: '自定义', value: 'custom' },
];

export function CompressPanel({ value, current, onChange }: Props) {
  const set = (patch: Partial<CompressSettings>) => onChange({ ...value, ...patch });
  const setResize = (patch: Partial<CompressSettings['resize']>) => set({ resize: { ...value.resize, ...patch } });
  const r = value.resize;
  const ratio = current?.info.width && current.info.height ? current.info.width / current.info.height : 1;

  // 自定义 + 锁定比例时，另一边按当前图片比例联动显示
  const onWidth = (w: number | null) => {
    if (r.mode === 'custom' && r.lock) setResize({ width: w, height: w ? Math.round(w / ratio) : null });
    else setResize({ width: w });
  };
  const onHeight = (h: number | null) => {
    if (r.mode === 'custom' && r.lock) setResize({ height: h, width: h ? Math.round(h * ratio) : null });
    else setResize({ height: h });
  };

  return (
    <div className="panel-section">
      <div className="panel-title">
        压缩方式 <span className="panel-hint">SVG 会渲染为 PNG；不支持 heic</span>
      </div>
      <Radio.Group className="mode-group" value={value.mode} onChange={(e) => set({ mode: e.target.value })}>
        <Radio value="normal">
          <div className="mode-name">普通压缩</div>
          <div className="mode-desc">压缩比例小，体积较大，画质几乎无损</div>
        </Radio>
        <Radio value="high">
          <div className="mode-name">高品质压缩</div>
          <div className="mode-desc">在保证清晰度的前提下，压缩到最小</div>
        </Radio>
        <Radio value="target">
          <div className="mode-name">每张图片体积小于</div>
        </Radio>
        {value.mode === 'target' && (
          <Space.Compact className="indent" block>
            <InputNumber
              aria-label="目标体积"
              min={1}
              max={value.targetUnit === 'MB' ? 100 : 102400}
              value={value.targetValue}
              onChange={(v) => set({ targetValue: v ?? 1 })}
              style={{ flex: 1 }}
            />
            <Select
              aria-label="体积单位"
              value={value.targetUnit}
              onChange={(u) => set({ targetUnit: u })}
              options={[{ value: 'KB' }, { value: 'MB' }]}
              style={{ width: 72 }}
            />
          </Space.Compact>
        )}
        <Radio value="custom">
          <div className="mode-name">自定义质量</div>
        </Radio>
        {value.mode === 'custom' && (
          <div className="indent slider-row">
            <Slider min={1} max={100} value={value.quality} onChange={(q) => set({ quality: q })} style={{ flex: 1 }} />
            <InputNumber aria-label="质量" min={1} max={100} value={value.quality} onChange={(q) => set({ quality: q ?? 80 })} style={{ width: 64 }} />
          </div>
        )}
      </Radio.Group>

      <div className="panel-title">分辨率（单位：像素）</div>
      <Segmented block size="small" options={RESIZE_OPTIONS} value={r.mode} onChange={(m) => setResize({ mode: m as ResizeMode })} />
      {r.mode === 'original' && current && (
        <div className="panel-hint block">
          当前图片 {current.info.width} × {current.info.height}
        </div>
      )}
      {r.mode === 'percent' && (
        <div className="slider-row">
          <Slider min={5} max={100} value={r.percent} onChange={(p) => setResize({ percent: p })} style={{ flex: 1 }} />
          <InputNumber aria-label="缩放百分比" min={5} max={100} value={r.percent} suffix="%" onChange={(p) => setResize({ percent: p ?? 100 })} style={{ width: 78 }} />
        </div>
      )}
      {(r.mode === 'max' || r.mode === 'custom') && (
        <div className="size-row">
          <InputNumber aria-label="宽" min={1} max={20000} value={r.width} onChange={onWidth} suffix="宽" style={{ flex: 1 }} />
          {r.mode === 'custom' ? (
            <Tooltip title={r.lock ? '已锁定宽高比' : '未锁定宽高比'}>
              <button className="lock-btn" aria-pressed={r.lock} aria-label="锁定宽高比" onClick={() => setResize({ lock: !r.lock })}>
                {r.lock ? <LinkOutlined /> : <DisconnectOutlined />}
              </button>
            </Tooltip>
          ) : (
            <span className="lock-btn static">×</span>
          )}
          <InputNumber aria-label="高" min={1} max={20000} value={r.height} onChange={onHeight} suffix="高" style={{ flex: 1 }} />
        </div>
      )}
      {r.mode === 'max' && <div className="panel-hint block">等比缩放到不超过该宽高，小图不放大</div>}

      <div className="panel-title">输出格式</div>
      <Select
        aria-label="输出格式"
        style={{ width: '100%' }}
        value={value.format}
        onChange={(f) => set({ format: f })}
        options={[{ value: 'original', label: '保持原格式' }, ...OUTPUT_FORMATS.map((f) => ({ value: f, label: FORMAT_LABEL[f] }))]}
      />
    </div>
  );
}
