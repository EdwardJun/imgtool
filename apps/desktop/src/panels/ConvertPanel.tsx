import { InputNumber, Slider, Switch } from 'antd';
import type { ConvertSettings } from '../types';
import { FORMAT_LABEL, OUTPUT_FORMATS } from '../types';

interface Props {
  value: ConvertSettings;
  onChange: (v: ConvertSettings) => void;
}

const DESC: Record<string, string> = {
  jpeg: '兼容性最好，不支持透明',
  png: '支持透明，适合图标和截图',
  webp: '体积小，现代浏览器均支持',
  avif: '体积最小，编码较慢',
};

export function ConvertPanel({ value, onChange }: Props) {
  const set = (patch: Partial<ConvertSettings>) => onChange({ ...value, ...patch });
  const losslessAvailable = value.format === 'png' || value.format === 'webp';

  return (
    <div className="panel-section">
      <div className="panel-title">
        目标格式 <span className="panel-hint">支持 jpg/png/webp/gif/bmp/tiff 输入</span>
      </div>
      <div className="format-grid">
        {OUTPUT_FORMATS.map((f) => (
          <button key={f} className={`format-card ${value.format === f ? 'active' : ''}`} aria-pressed={value.format === f} onClick={() => set({ format: f })}>
            <div className="format-name">{FORMAT_LABEL[f]}</div>
            <div className="format-desc">{DESC[f]}</div>
          </button>
        ))}
      </div>

      {losslessAvailable && (
        <div className="switch-row">
          <span>无损转换</span>
          <Switch aria-label="无损转换" checked={value.lossless} onChange={(v) => set({ lossless: v })} />
        </div>
      )}
      {!(losslessAvailable && value.lossless) && (
        <>
          <div className="panel-title">质量</div>
          <div className="slider-row">
            <Slider min={1} max={100} value={value.quality} onChange={(q) => set({ quality: q })} style={{ flex: 1 }} />
            <InputNumber aria-label="质量" min={1} max={100} value={value.quality} onChange={(q) => set({ quality: q ?? 80 })} style={{ width: 64 }} />
          </div>
          {value.format === 'png' && <div className="panel-hint block">PNG 质量低于 95 时使用调色板有损压缩</div>}
        </>
      )}
    </div>
  );
}
