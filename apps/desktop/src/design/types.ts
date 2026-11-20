/** 设计解析 spec（与 crates/imgspec/src/model.rs 一致） */

export type NodeType = 'frame' | 'rect' | 'text' | 'icon' | 'image' | 'line';
export type NodeSource = 'auto' | 'agent' | 'manual';
export type SpecKind = 'page' | 'component';

export interface Bounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

export type Fill = { type: 'solid'; color: string } | { type: 'linear'; angle: number; stops: { pos: number; color: string }[] };

export interface NodeStyle {
  fill?: Fill;
  stroke?: { color: string; width: number };
  corner_radius?: number | [number, number, number, number];
  shadow?: { x: number; y: number; blur: number; color: string; confidence: number };
}

export interface TextStyle {
  color: string;
  font_size: number;
  line_height?: number;
  lines: number;
  weight: 'regular' | 'medium' | 'bold';
  align: 'left' | 'center' | 'right';
  confidence: number;
}

export interface Layout {
  mode: 'row' | 'column' | 'none';
  gap?: number;
  gap_uniform: boolean;
  padding: [number, number, number, number];
  align: 'start' | 'center' | 'end' | 'space_between';
  cross_align: 'start' | 'center' | 'end' | 'stretch' | 'mixed';
}

export interface Asset {
  path: string;
  source: 'crop' | 'generated';
  width: number;
  height: number;
}

export interface SpecNode {
  id: string;
  type: NodeType;
  role?: string;
  name?: string;
  text?: string;
  description?: string;
  bounds: Bounds;
  relative: { x: number; y: number };
  style?: NodeStyle;
  text_style?: TextStyle;
  layout?: Layout;
  asset?: Asset;
  confidence: number;
  source: NodeSource;
  children?: SpecNode[];
}

export interface Spec {
  version: number;
  generator: string;
  source: { path: string; width: number; height: number };
  kind: SpecKind;
  background?: Fill;
  transparent: boolean;
  palette: { color: string; ratio: number }[];
  annotated: boolean;
  notes?: string;
  root: SpecNode;
}

export interface IconFile {
  id: string;
  name: string;
  path: string;
  source: 'crop' | 'generated';
  size: number;
}

export interface DesignResult {
  spec: Spec;
  root: string;
  /** 这张图的解析文件夹：spec、标注图、图标素材都在里面 */
  dir: string;
  spec_path: string;
  overlay_path: string;
  icons: IconFile[];
  warnings: string[];
  kept_annotations: number;
}

export interface RunOptions {
  analyze: { mode: SpecKind | null; tolerance: number | null; max_depth: number; max_edge: number };
  keep_annotations: boolean;
  export_icons: boolean;
  icons: { size: number; padding: number };
}

/** 设计解析页的设置（localStorage） */
export interface DesignSettings {
  mode: 'auto' | SpecKind;
  /** null = 自动 */
  tolerance: number | null;
  exportIcons: boolean;
  /** 叠加层显示到第几层（0 = 全部） */
  depth: number;
  showGroups: boolean;
  iconStyle: string;
  iconModel: string;
  /** 也重绘原图过小的图标 */
  iconForceSmall: boolean;
}

export const defaultDesignSettings: DesignSettings = {
  mode: 'auto',
  tolerance: null,
  exportIcons: true,
  depth: 0,
  showGroups: false,
  iconStyle: '',
  iconModel: 'gpt-image-2',
  iconForceSmall: false,
};

export const NODE_TYPES: { value: NodeType; label: string }[] = [
  { value: 'frame', label: '容器 frame' },
  { value: 'rect', label: '色块 rect' },
  { value: 'text', label: '文字 text' },
  { value: 'icon', label: '图标 icon' },
  { value: 'image', label: '图片 image' },
  { value: 'line', label: '分割线 line' },
];

export const ROLE_OPTIONS = ['page', 'header', 'nav_bar', 'tab_bar', 'tab_item', 'card', 'list', 'list_item', 'button', 'input', 'search', 'title', 'subtitle', 'paragraph', 'price', 'tag', 'badge', 'avatar', 'banner', 'divider', 'icon'];
