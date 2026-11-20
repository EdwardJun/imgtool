import { Button, Tooltip } from 'antd';
import { CompressOutlined, DragOutlined, ZoomInOutlined, ZoomOutOutlined } from '@ant-design/icons';
import { clampZoom } from './FitBox';

interface Props {
  /** 相对"适应窗口"的倍数 */
  zoom: number;
  /** 适应窗口时的显示比例（显示像素 / 原图像素） */
  fit: number;
  onChange: (zoom: number) => void;
  /** 抓手模式：直接拖动画面 */
  panMode: boolean;
  onPanModeChange: (on: boolean) => void;
}

const STEP = 1.25;

/** 预览区的放大 / 缩小控件：百分比为相对原图像素的实际比例 */
export function ZoomBar({ zoom, fit, onChange, panMode, onPanModeChange }: Props) {
  const pct = Math.round(zoom * fit * 100);
  return (
    <div className="zoom-bar" role="toolbar" aria-label="缩放">
      <Tooltip title={panMode ? '抓手：拖动画面（再点一次回到编辑框）' : '抓手：拖动画面。也可以按住空格或鼠标中键拖动'}>
        <Button size="small" type={panMode ? 'primary' : 'text'} icon={<DragOutlined />} aria-label="抓手" aria-pressed={panMode} onClick={() => onPanModeChange(!panMode)} />
      </Tooltip>
      <Tooltip title="缩小（⌘ -）">
        <Button size="small" type="text" icon={<ZoomOutOutlined />} aria-label="缩小" onClick={() => onChange(clampZoom(zoom / STEP))} />
      </Tooltip>
      <Tooltip title="点击切换：适应窗口 / 原图 100%（⌘ 0）。按住 ⌘ 或 Ctrl 滚动、触控板双指捏合也可以缩放">
        <Button size="small" type="text" className="zoom-value" onClick={() => onChange(Math.abs(zoom - 1) < 0.01 && fit < 1 ? clampZoom(1 / fit) : 1)}>
          {pct}%
        </Button>
      </Tooltip>
      <Tooltip title="放大（⌘ +）">
        <Button size="small" type="text" icon={<ZoomInOutlined />} aria-label="放大" onClick={() => onChange(clampZoom(zoom * STEP))} />
      </Tooltip>
      <Tooltip title="适应窗口">
        <Button size="small" type="text" icon={<CompressOutlined />} aria-label="适应窗口" disabled={Math.abs(zoom - 1) < 0.01} onClick={() => onChange(1)} />
      </Tooltip>
    </div>
  );
}
