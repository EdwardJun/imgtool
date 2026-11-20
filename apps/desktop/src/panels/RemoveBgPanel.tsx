import { useEffect, useState } from 'react';
import { Alert, App, Button, ColorPicker, InputNumber, Progress, Radio, Segmented, Slider, Switch, Tooltip } from 'antd';
import { QuestionCircleOutlined } from '@ant-design/icons';
import { listen } from '@tauri-apps/api/event';
import { api } from '../api';
import type { ImageItem, MattingModelStatus, PhotoSizeKey, RemoveBgSettings } from '../types';
import { defaultRemoveBg, FILL_PRESETS, frameKeyOf, PHOTO_SIZES, photoSizeOf, rgbToHex } from '../options';

interface Props {
  value: RemoveBgSettings;
  current?: ImageItem;
  onChange: (v: RemoveBgSettings) => void;
  /** 正在左侧拖框微调证件照构图 */
  framing: boolean;
  onToggleFraming: () => void;
  /** 丢弃当前图片手动调整的构图，恢复自动构图 */
  onResetFrame: () => void;
  /** AI 模型下载完成，需要重新预览 */
  onModelReady: () => void;
}

/** AI 抠图模型状态：未下载时提供下载按钮和进度 */
function ModelStatus({ onReady }: { onReady: () => void }) {
  const { message } = App.useApp();
  const [status, setStatus] = useState<MattingModelStatus | null>(null);
  const [progress, setProgress] = useState<number | null>(null);

  useEffect(() => {
    api.mattingModelStatus().then(setStatus).catch(() => undefined);
  }, []);

  const download = async () => {
    setProgress(0);
    const unlisten = await listen<{ downloaded: number; total: number }>('model-download-progress', (e) => {
      setProgress(Math.round((e.payload.downloaded / Math.max(1, e.payload.total)) * 100));
    });
    try {
      const s = await api.downloadMattingModel();
      setStatus(s);
      message.success('AI 人像抠图模型已下载');
      onReady();
    } catch (e) {
      message.error(String(e));
    } finally {
      unlisten();
      setProgress(null);
    }
  };

  if (!status || status.available) return null;
  return (
    <Alert
      className="model-alert"
      type="info"
      showIcon
      title={`首次使用需下载模型（约 ${Math.round(status.bytes / 1024 / 1024)}MB）`}
      description={
        <div className="model-alert-body">
          <span>下载一次后离线可用，MCP 也用同一份。许可：{status.license}</span>
          {progress !== null ? (
            <Progress percent={progress} size="small" aria-label="模型下载进度" />
          ) : (
            <Button size="small" type="primary" block onClick={download}>
              下载模型
            </Button>
          )}
        </div>
      }
    />
  );
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

const SIZE_CHIPS: { key: PhotoSizeKey; label: string; hint?: string }[] = [
  { key: 'original', label: '原尺寸' },
  ...PHOTO_SIZES.map((p) => ({ key: p.key, label: p.label, hint: `${p.width}×${p.height}` })),
  { key: 'custom', label: '自定义' },
];

/** 背景处理参数面板：透明背景 / 更换底色（证件照）。设置会被记住，生成图片时复用去背景参数 */
export function RemoveBgPanel({ value, current, onChange, framing, onToggleFraming, onResetFrame, onModelReady }: Props) {
  const set = (p: Partial<RemoveBgSettings>) => onChange({ ...value, ...p });
  const detected = current?.preview?.report.background;
  const replace = value.mode === 'replace';
  const size = photoSizeOf(value);
  const hasFrame = !!(size && current?.frame && current.frameKey === frameKeyOf(size));
  const method = replace ? value.fillMethod : value.method;
  const ai = method === 'ai';

  return (
    <div className="panel-section">
      <Segmented
        block
        className="crop-mode"
        value={value.mode}
        onChange={(m) => set({ mode: m as RemoveBgSettings['mode'] })}
        options={[
          { label: '透明背景', value: 'transparent' },
          { label: '更换底色', value: 'replace' },
        ]}
      />
      <div className="panel-hint block">{replace ? '把背景换成新底色，可输出一寸、二寸等证件照。' : '去除背景，输出透明图片。'}</div>

      <div className="panel-title">抠图方式</div>
      <Radio.Group value={method} onChange={(e) => set(replace ? { fillMethod: e.target.value } : { method: e.target.value })}>
        <Radio value="ai">AI 人像</Radio>
        <Radio value="color">按颜色</Radio>
      </Radio.Group>
      <div className="panel-hint block">
        {ai ? '本地 MODNet 模型，只适合人物照片；背景不必是纯色，发丝更自然，白衬衫不会被误删。' : '按背景颜色抠图，适合纯色背景的产品图、图标和人像。'}
      </div>
      {ai && <ModelStatus onReady={onModelReady} />}

      <div className="panel-title">{replace ? '原背景颜色' : '背景颜色'}</div>
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

      <SliderRow
        label="容差"
        tip={ai ? 'AI 抠图时只用于清理人像边缘：模型拿不准的边缘像素，颜色在这个范围内接近原背景的就去掉，避免出现一圈光晕。' : '颜色与背景色差距在这个范围内的像素会被完全去除。背景有渐变或噪点时调大，主体被误删时调小。'}
        value={value.tolerance}
        max={60}
        onChange={(v) => set({ tolerance: v })}
      />
      <SliderRow label="边缘柔和" tip="容差外再往外这一段做半透明过渡，让边缘不出现锯齿。" value={value.softness} max={40} onChange={(v) => set({ softness: v })} />

      {!ai && <div className="switch-row">
        <span>
          只去除与边缘相连的背景{' '}
          <Tooltip title="开启时，主体内部与背景同色的区域（如白色文字、眼白、白衬衫）会保留；关闭则全图同色像素都去除。">
            <QuestionCircleOutlined className="muted" />
          </Tooltip>
        </span>
        <Switch aria-label="只去除与边缘相连的背景" checked={value.contiguous} onChange={(v) => set({ contiguous: v })} />
      </div>}
      <div className="switch-row">
        <span>
          去除白边{' '}
          <Tooltip title="把半透明边缘里混入的原背景色去掉，换到深色底或新底色上不会出现一圈白边。">
            <QuestionCircleOutlined className="muted" />
          </Tooltip>
        </span>
        <Switch aria-label="去除白边" checked={value.decontaminate} onChange={(v) => set({ decontaminate: v })} />
      </div>

      {replace && !ai && (
        <div className="switch-row">
          <span>
            人像模式{' '}
            <Tooltip title="只去除从图片左、右、上边缘能直接看到的背景，被人物挡住的同色区域（如西装里的白衬衫、领口）会保留。拍的不是人像时可以关掉。">
              <QuestionCircleOutlined className="muted" />
            </Tooltip>
          </span>
          <Switch aria-label="人像模式" checked={value.portrait} onChange={(v) => set({ portrait: v })} />
        </div>
      )}

      {replace ? (
        <>
          <div className="panel-title">新底色</div>
          <div className="switch-row fill-row">
            {FILL_PRESETS.map((f) => (
              <button
                key={f.color}
                className={`chip fill-chip ${value.fill.toLowerCase() === f.color ? 'active' : ''}`}
                aria-pressed={value.fill.toLowerCase() === f.color}
                onClick={() => set({ fill: f.color })}
              >
                <span className="swatch" style={{ background: f.color }} />
                {f.label}
              </button>
            ))}
            <ColorPicker aria-label="自定义底色" value={value.fill} disabledAlpha showText onChangeComplete={(c) => set({ fill: c.toHexString().slice(0, 7) })} />
          </div>

          <div className="panel-title">证件照尺寸</div>
          <div className="chip-grid size-grid">
            {SIZE_CHIPS.map((s) => (
              <button key={s.key} className={`chip ${value.photoSize === s.key ? 'active' : ''}`} aria-pressed={value.photoSize === s.key} onClick={() => set({ photoSize: s.key })}>
                {s.label}
                {s.hint && <span className="chip-hint">{s.hint}</span>}
              </button>
            ))}
          </div>
          {value.photoSize === 'custom' && (
            <div className="size-row">
              <InputNumber aria-label="证件照宽" min={1} max={10000} value={value.customWidth} onChange={(v) => set({ customWidth: v })} suffix="宽" style={{ flex: 1 }} />
              <span className="lock-btn static">×</span>
              <InputNumber aria-label="证件照高" min={1} max={10000} value={value.customHeight} onChange={(v) => set({ customHeight: v })} suffix="高" style={{ flex: 1 }} />
            </div>
          )}
          {size ? (
            <>
              <div className="panel-hint block">
                按人像自动构图：头部水平居中、头顶留白，再缩放到 {size.width}×{size.height} 像素。
                {hasFrame ? '当前图片已手动调整构图。' : ''}
              </div>
              <div className="switch-row">
                <Button size="small" type={framing ? 'primary' : 'default'} disabled={!current} onClick={onToggleFraming}>
                  {framing ? '完成调整' : '调整构图'}
                </Button>
                {hasFrame && (
                  <Button size="small" type="link" onClick={onResetFrame}>
                    恢复自动构图
                  </Button>
                )}
              </div>
            </>
          ) : (
            value.photoSize === 'custom' && <div className="panel-hint block">请填写宽和高（像素）</div>
          )}

          <div className="switch-row">
            <span>
              限制文件大小{' '}
              <Tooltip title="很多报名网站要求照片不超过某个体积（如 50KB）。只会降低质量，不会缩小分辨率。">
                <QuestionCircleOutlined className="muted" />
              </Tooltip>
            </span>
            <Switch aria-label="限制文件大小" checked={value.limitSize} onChange={(v) => set({ limitSize: v })} />
          </div>
          {value.limitSize && (
            <div className="slider-row">
              <span className="muted">不超过</span>
              <InputNumber aria-label="文件大小上限" min={1} max={10240} value={value.maxKb} suffix="KB" onChange={(v) => set({ maxKb: v ?? 50 })} style={{ flex: 1 }} />
            </div>
          )}

          <div className="panel-title">输出格式</div>
          <Radio.Group value={value.fillFormat} onChange={(e) => set({ fillFormat: e.target.value })}>
            <Radio value="jpeg">JPG</Radio>
            <Radio value="png">PNG</Radio>
          </Radio.Group>
        </>
      ) : (
        <>
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
          <div className="panel-hint block">JPG 不支持透明，所以不提供；需要 JPG 请用「更换底色」。</div>
        </>
      )}

      <Button size="small" onClick={() => onChange({ ...defaultRemoveBg, mode: value.mode })}>
        恢复默认参数
      </Button>
    </div>
  );
}
