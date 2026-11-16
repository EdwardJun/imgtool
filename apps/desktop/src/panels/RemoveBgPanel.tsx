import { Button, ColorPicker, InputNumber, Radio, Slider, Switch, Tooltip } from 'antd';
import { QuestionCircleOutlined } from '@ant-design/icons';
import type { ImageItem, RemoveBgSettings } from '../types';
import { defaultRemoveBg, rgbToHex } from '../options';

interface Props {
  value: RemoveBgSettings;
  current?: ImageItem;
  onChange: (v: RemoveBgSettings) => void;
}

function SliderRow({ label, tip, value, max, onChange }: { label: string; tip: string; value: number; max: number; onChange: (v: number) => void }) {
  return (
    <>
      <div className="panel-title">
        {label}{' '}
        <Tooltip title={tip}>
          <QuestionCircleOutlined className="muted" />
        </Tooltip>
      </div>
      <div className="slider-row">
        <Slider min={0} max={max} value={value} onChange={onChange} style={{ flex: 1 }} />
        <InputNumber aria-label={label} min={0} max={max} value={value} onChange={(v) => onChange(v ?? 0)} style={{ width: 64 }} />
      </div>
    </>
  );
}

/** 去除背景的参数面板，设置会被记住，生成图片时也复用这份设置 */
export function RemoveBgPanel({ value, current, onChange }: Props) {
  const set = (p: Partial<RemoveBgSettings>) => onChange({ ...value, ...p });
  const detected = current?.preview?.report.background;

  return (
    <div className="panel-section">
      <div className="panel-title">
        去除背景 <span className="panel-hint">适合纯色或近纯色背景</span>
      </div>

      <div className="panel-title">背景颜色</div>
      <Radio.Group value={value.auto ? 'auto' : 'manual'} onChange={(e) => set({ auto: e.target.value === 'auto' })}>
        <Radio value="auto">自动识别</Radio>
        <Radio value="manual">指定颜色</Radio>
      </Radio.Group>
      {value.auto ? (
        detected && (
          <div className="bg-detected">
            <span className="swatch" style={{ background: rgbToHex(detected.color) }} />
            识别为 {rgbToHex(detected.color)}，边缘匹配 {Math.round(detected.border_match * 100)}%
            <Button size="small" type="link" onClick={() => set({ auto: false, color: rgbToHex(detected.color) })}>
              改为指定
            </Button>
          </div>
        )
      ) : (
        <div className="switch-row">
          <ColorPicker
            aria-label="背景颜色"
            value={value.color}
            disabledAlpha
            showText
            onChangeComplete={(c) => set({ color: c.toHexString().slice(0, 7) })}
          />
          <span className="panel-hint">可点取色器里的吸管从图上取色</span>
        </div>
      )}

      <SliderRow label="容差" tip="颜色与背景色差距在这个范围内的像素会被完全去除。背景有渐变或噪点时调大，主体被误删时调小。" value={value.tolerance} max={60} onChange={(v) => set({ tolerance: v })} />
      <SliderRow label="边缘柔和" tip="容差外再往外这一段做半透明过渡，让边缘不出现锯齿。" value={value.softness} max={40} onChange={(v) => set({ softness: v })} />

      <div className="switch-row">
        <span>
          只去除与边缘相连的背景{' '}
          <Tooltip title="开启时，主体内部与背景同色的区域（如白色文字、眼白）会保留；关闭则全图同色像素都去除。">
            <QuestionCircleOutlined className="muted" />
          </Tooltip>
        </span>
        <Switch aria-label="只去除与边缘相连的背景" checked={value.contiguous} onChange={(v) => set({ contiguous: v })} />
      </div>
      <div className="switch-row">
        <span>
          去除白边{' '}
          <Tooltip title="把半透明边缘里混入的背景色去掉，放到深色背景上不会出现一圈白边。">
            <QuestionCircleOutlined className="muted" />
          </Tooltip>
        </span>
        <Switch aria-label="去除白边" checked={value.decontaminate} onChange={(v) => set({ decontaminate: v })} />
      </div>
      <div className="switch-row">
        <span>裁掉透明空白</span>
        <Switch aria-label="裁掉透明空白" checked={value.trim} onChange={(v) => set({ trim: v })} />
      </div>
      {value.trim && (
        <div className="slider-row">
          <span className="muted">保留边距</span>
          <InputNumber aria-label="保留边距" min={0} max={500} value={value.trimPadding} suffix="px" onChange={(v) => set({ trimPadding: v ?? 0 })} style={{ flex: 1 }} />
        </div>
      )}

      <div className="panel-title">输出格式</div>
      <Radio.Group value={value.format} onChange={(e) => set({ format: e.target.value })}>
        <Radio value="png">PNG</Radio>
        <Radio value="webp">WebP</Radio>
        <Radio value="avif">AVIF</Radio>
      </Radio.Group>
      <div className="panel-hint block">JPG 不支持透明，所以不提供。</div>

      <Button size="small" onClick={() => onChange(defaultRemoveBg)}>
        恢复默认参数
      </Button>
    </div>
  );
}
