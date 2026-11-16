import { Button, Checkbox, Empty, Input, InputNumber, Radio, Select, Slider, Spin, Switch, Tooltip } from 'antd';
import { DeleteOutlined, QuestionCircleOutlined, ReloadOutlined } from '@ant-design/icons';
import { fileSrc } from '../api';
import type { ImageItem, Region, SmartCropSettings } from '../types';

interface Props {
  value: SmartCropSettings;
  current?: ImageItem;
  detecting: boolean;
  selected: number | null;
  onChange: (v: SmartCropSettings) => void;
  onSelect: (i: number | null) => void;
  onRegionsChange: (regions: Region[], names: string[]) => void;
  onRedetect: () => void;
}

const SIZES = [128, 256, 512, 1024];
const THUMB = 36;

function Thumb({ item, region }: { item: ImageItem; region: Region }) {
  const W = item.info.width ?? 1;
  const H = item.info.height ?? 1;
  const s = THUMB / Math.max(region.width, region.height);
  const ox = (THUMB - region.width * s) / 2;
  const oy = (THUMB - region.height * s) / 2;
  return (
    <span
      className="region-thumb checkerboard"
      style={{
        backgroundImage: `url("${fileSrc(item.display_path)}")`,
        backgroundSize: `${W * s}px ${H * s}px`,
        backgroundPosition: `${ox - region.x * s}px ${oy - region.y * s}px`,
      }}
    />
  );
}

/** 智能裁剪：识别参数、导出尺寸、识别结果列表（可改名、删除） */
export function SmartCropPanel({ value, current, detecting, selected, onChange, onSelect, onRegionsChange, onRedetect }: Props) {
  const set = (p: Partial<SmartCropSettings>) => onChange({ ...value, ...p });
  const regions = current?.regions ?? [];
  const names = current?.regionNames ?? [];
  const info = current?.detectInfo;
  const custom = !SIZES.includes(value.size);

  const rename = (i: number, name: string) => {
    const next = regions.map((_, j) => (j === i ? name : names[j] ?? ''));
    onRegionsChange(regions, next);
  };
  const remove = (i: number) => {
    onRegionsChange(
      regions.filter((_, j) => j !== i),
      names.filter((_, j) => j !== i),
    );
    onSelect(null);
  };

  return (
    <div className="panel-section">
      <div className="switch-row">
        <span>
          包含图标下方的文字 <span className="panel-hint">默认关闭，只保留图标</span>
        </span>
        <Switch aria-label="包含图标下方的文字" checked={value.includeLabels} onChange={(v) => set({ includeLabels: v })} />
      </div>

      <div className="panel-title">
        合并间距{' '}
        <Tooltip title="距离小于这个值的笔画算作同一个图标。图标被拆成多块时调大，相邻图标被合成一个时调小。">
          <QuestionCircleOutlined className="muted" />
        </Tooltip>
      </div>
      <div className="slider-row">
        <Checkbox checked={value.gap === null} onChange={(e) => set({ gap: e.target.checked ? null : info?.gap ?? 12 })}>
          自动{value.gap === null && info ? `（${info.gap}px）` : ''}
        </Checkbox>
        {value.gap !== null && (
          <InputNumber aria-label="合并间距" min={0} max={300} value={value.gap} suffix="px" onChange={(v) => set({ gap: v ?? 0 })} style={{ flex: 1 }} />
        )}
      </div>

      <div className="panel-title">
        最小尺寸{' '}
        <Tooltip title="长边小于这个值的区域视为噪点，不会被识别成图标。">
          <QuestionCircleOutlined className="muted" />
        </Tooltip>
      </div>
      <div className="slider-row">
        <Checkbox checked={value.minSize === null} onChange={(e) => set({ minSize: e.target.checked ? null : info?.min_size ?? 8 })}>
          自动{value.minSize === null && info ? `（${info.min_size}px）` : ''}
        </Checkbox>
        {value.minSize !== null && (
          <InputNumber aria-label="最小尺寸" min={1} max={2000} value={value.minSize} suffix="px" onChange={(v) => set({ minSize: v ?? 1 })} style={{ flex: 1 }} />
        )}
      </div>

      <div className="panel-title">导出尺寸（正方形，图标居中）</div>
      <div className="slider-row">
        <Select
          aria-label="导出尺寸"
          value={custom ? 'custom' : value.size}
          onChange={(v) => set({ size: v === 'custom' ? 300 : (v as number) })}
          options={[...SIZES.map((s) => ({ value: s, label: `${s} × ${s}` })), { value: 'custom', label: '自定义' }]}
          style={{ flex: 1 }}
        />
        {custom && <InputNumber aria-label="自定义尺寸" min={8} max={4096} value={value.size} suffix="px" onChange={(v) => set({ size: v ?? 256 })} style={{ width: 110 }} />}
      </div>
      <div className="panel-title">四周留白</div>
      <div className="slider-row">
        <Slider min={0} max={40} value={value.padding} onChange={(v) => set({ padding: v })} style={{ flex: 1 }} />
        <InputNumber aria-label="四周留白" min={0} max={40} value={value.padding} suffix="%" onChange={(v) => set({ padding: v ?? 0 })} style={{ width: 78 }} />
      </div>
      <div className="switch-row">
        <span>输出格式</span>
        <Radio.Group value={value.format} onChange={(e) => set({ format: e.target.value })}>
          <Radio value="png">PNG</Radio>
          <Radio value="webp">WebP</Radio>
        </Radio.Group>
      </div>

      <div className="panel-title switch-row">
        <span>
          识别结果 {current && !detecting && <span className="panel-hint">{regions.length} 个{current.regionsEdited ? '（已手动调整）' : ''}</span>}
        </span>
        <Button size="small" icon={<ReloadOutlined />} onClick={onRedetect} disabled={!current} loading={detecting}>
          重新识别
        </Button>
      </div>
      {current?.regionsError && <div className="stats-error">{current.regionsError}</div>}
      {detecting && !regions.length ? (
        <Spin />
      ) : regions.length ? (
        <div className="region-list">
          {regions.map((r, i) => (
            <div key={i} className={`region-row ${i === selected ? 'selected' : ''}`} onClick={() => onSelect(i)}>
              <span className="region-no">{i + 1}</span>
              {current && <Thumb item={current} region={r} />}
              <Input
                size="small"
                aria-label={`第 ${i + 1} 个图标的文件名`}
                placeholder={`icon_${String(i + 1).padStart(2, '0')}`}
                value={names[i] ?? ''}
                onChange={(e) => rename(i, e.target.value)}
              />
              <Button size="small" type="text" icon={<DeleteOutlined />} aria-label={`删除第 ${i + 1} 个`} onClick={(e) => (e.stopPropagation(), remove(i))} />
            </div>
          ))}
        </div>
      ) : (
        current && <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="没有识别到图标，可调整参数或在图上拖出框" />
      )}
      <div className="panel-hint block">在左侧图上拖动可新增框；选中框后拖四角调整，按 Delete 删除。适合透明底或纯色底、图标之间有空隙的图。</div>
    </div>
  );
}
