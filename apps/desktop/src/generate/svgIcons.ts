import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

export type IconVariant = 'outline' | 'filled';
export type IconCorner = 'round' | 'square';

export interface IconStyle {
  variant: IconVariant;
  /** 24×24 画布上的线宽 */
  stroke_width: number;
  corner: IconCorner;
  notes: string | null;
}

export interface SvgRenderOptions {
  color: string;
  sizes: number[];
  format: 'png' | 'webp';
}

export interface SvgExportFile {
  path: string;
  /** "svg" 或位图格式 */
  format: string;
  size?: number;
  bytes: number;
  written: boolean;
}

/** 后端 IconSetItem：成功时带 name/svg/files，失败时带 error */
export interface IconSetItem {
  label: string;
  name?: string;
  svg?: string;
  files?: SvgExportFile[];
  removed?: string[];
  warnings?: string[];
  error?: string;
}

export interface SvgIconSet {
  model: string | null;
  total_tokens: number | null;
  elapsed_ms: number;
  out_dir: string;
  icons: IconSetItem[];
  missing: string[];
}

export interface SvgForm {
  /** 每行一个图标，可写"名称：说明" */
  list: string;
  style: IconStyle;
  render: SvgRenderOptions;
}

export const DEFAULT_ICON_COLOR = '#1F2937';
export const SIZE_PRESETS = [16, 24, 32, 48, 64, 96, 128, 256, 512];
export const MAX_ICONS = 12;

const FORM_KEY = 'imgtool.svgIcons.v1';
const DEFAULT_FORM: SvgForm = {
  list: '首页\n发现\n购物车\n我的',
  style: { variant: 'outline', stroke_width: 2, corner: 'round', notes: null },
  render: { color: DEFAULT_ICON_COLOR, sizes: [64, 128, 256], format: 'png' },
};

export function loadSvgForm(): SvgForm {
  try {
    const saved = JSON.parse(localStorage.getItem(FORM_KEY) ?? '{}');
    return { ...DEFAULT_FORM, ...saved, style: { ...DEFAULT_FORM.style, ...saved.style }, render: { ...DEFAULT_FORM.render, ...saved.render } };
  } catch {
    return DEFAULT_FORM;
  }
}
export const saveSvgForm = (f: SvgForm) => localStorage.setItem(FORM_KEY, JSON.stringify(f));

export const parseList = (list: string) =>
  list
    .split(/\n|[,，、]/)
    .map((s) => s.trim())
    .filter(Boolean);

export const svgApi = {
  generate: (params: { base_url: string; api_key: string; model: string; icons: string[]; style: IconStyle } & SvgRenderOptions) =>
    invoke<SvgIconSet>('generate_svg_icons', { params }),
  render: (params: { icons: { label: string; name: string; svg: string }[] } & SvgRenderOptions) => invoke<SvgIconSet>('render_svg_set', { params }),
};

export interface SvgBatch {
  id: string;
  status: 'running' | 'done' | 'failed';
  startedAt: number;
  finishedAt?: number;
  labels: string[];
  render: SvgRenderOptions;
  set?: SvgIconSet;
  error?: string;
}

/** SVG 图标的生成记录：生成、换色 / 改尺寸重新导出、删除 */
export function useSvgIcons() {
  const [batches, setBatches] = useState<SvgBatch[]>([]);
  const running = batches.some((b) => b.status === 'running');
  const patch = (id: string, p: Partial<SvgBatch>) => setBatches((bs) => bs.map((b) => (b.id === id ? { ...b, ...p } : b)));

  const generate = async (params: Parameters<typeof svgApi.generate>[0]) => {
    const id = `svg-${Date.now()}`;
    const render = { color: params.color, sizes: params.sizes, format: params.format };
    setBatches((bs) => [{ id, status: 'running', startedAt: Date.now(), labels: params.icons, render }, ...bs]);
    try {
      const set = await svgApi.generate(params);
      patch(id, { status: 'done', set, finishedAt: Date.now() });
      return set;
    } catch (e) {
      patch(id, { status: 'failed', error: String(e), finishedAt: Date.now() });
      throw e;
    }
  };

  /** 不调用模型，用已有 SVG 按新的颜色 / 尺寸 / 格式重新导出 */
  const rerender = async (id: string, render: SvgRenderOptions) => {
    const b = batches.find((x) => x.id === id);
    const icons = (b?.set?.icons ?? []).filter((i) => i.svg && i.name).map((i) => ({ label: i.label, name: i.name!, svg: i.svg! }));
    if (!b || !icons.length) return;
    const set = await svgApi.render({ icons, ...render });
    patch(id, { set: { ...set, model: b.set?.model ?? null, total_tokens: b.set?.total_tokens ?? null, missing: b.set?.missing ?? [] }, render });
  };

  const remove = (id: string) => setBatches((bs) => bs.filter((b) => b.id !== id));
  const clear = () => setBatches((bs) => bs.filter((b) => b.status === 'running'));
  return { batches, running, generate, rerender, remove, clear };
}
