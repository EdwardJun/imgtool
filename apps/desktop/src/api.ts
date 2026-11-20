import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import type {
  EnhanceResult,
  PromptContext,
  UploadedRef,
  DetectOptions,
  DetectResult,
  Region,
  SplitResult,
  SquareOptions,
  ApiSettings,
  GenerateParams,
  GeneratedFile,
  ExportSummary,
  ExportTarget,
  FoundPreset,
  LoadedImage,
  MattingModelStatus,
  McpClient,
  McpStatus,
  PreviewResult,
  ProcessOptions,
} from './types';
import type { Bounds, DesignResult, RunOptions, Spec, SpecNode } from './design/types';

export const IMAGE_EXTENSIONS = ['jpg', 'jpeg', 'png', 'webp', 'gif', 'bmp', 'tif', 'tiff', 'ico', 'svg'];

type LoadResult = { Ok: LoadedImage } | { Err: string };

export const api = {
  expandPaths: (paths: string[]) => invoke<string[]>('expand_paths', { paths }),
  async loadImages(paths: string[]) {
    const results = await invoke<LoadResult[]>('load_images', { paths });
    const images: LoadedImage[] = [];
    const errors: string[] = [];
    for (const r of results) {
      if ('Ok' in r) images.push(r.Ok);
      else errors.push(r.Err);
    }
    return { images, errors };
  },
  preview: (path: string, options: ProcessOptions, previous?: string) =>
    invoke<PreviewResult>('preview', { path, options, previous: previous ?? null }),
  exportImages: (items: { path: string; options: ProcessOptions }[], target: ExportTarget) =>
    invoke<ExportSummary>('export_images', { items, target }),
  findPreset: (path: string) => invoke<FoundPreset | null>('find_preset', { path }),
  mcpStatus: () => invoke<McpStatus>('mcp_status'),
  mcpInstall: (client: McpClient) => invoke<string>('mcp_install', { client }),
  initialPaths: () => invoke<string[]>('initial_paths'),
  mattingModelStatus: () => invoke<MattingModelStatus>('matting_model_status'),
  downloadMattingModel: () => invoke<MattingModelStatus>('download_matting_model'),
  getApiSettings: () => invoke<ApiSettings>('get_api_settings'),
  saveApiSettings: (settings: ApiSettings) => invoke<void>('save_api_settings', { settings }),
  generateImage: (params: GenerateParams) => invoke<GeneratedFile>('generate_image', { params }),
  saveFiles: (paths: string[], dir: string) => invoke<string[]>('save_files', { paths, dir }),
  enhancePrompt: (params: { base_url: string; api_key: string; model: string; prompt: string; context: PromptContext }) =>
    invoke<EnhanceResult>('enhance_prompt', { params }),
  uploadReference: (baseUrl: string, apiKey: string, path: string) =>
    invoke<UploadedRef>('upload_reference', { baseUrl, apiKey, path }),
  detectRegions: (path: string, options: DetectOptions) => invoke<DetectResult>('detect_regions', { path, options }),
  analyzeDesign: (path: string, options: RunOptions) => invoke<DesignResult>('analyze_design', { path, options }),
  loadDesignSpec: (path: string) => invoke<DesignResult | null>('load_design_spec', { path }),
  saveDesignSpec: (path: string, spec: Spec) => invoke<DesignResult>('save_design_spec', { path, spec }),
  reanalyzeRegion: (path: string, bounds: Bounds) => invoke<SpecNode>('reanalyze_region', { path, bounds }),
  normalizeSpec: (spec: Spec) => invoke<Spec>('normalize_spec', { spec }),
  generateDesignIcons: (params: { path: string; base_url: string; api_key: string; model: string; quality?: string; ids?: string[] | null; style?: string | null }) =>
    invoke<DesignResult>('generate_design_icons', { params }),
  openFolder: (path: string) => invoke<void>('open_folder', { path }),
  async exportRegions(
    items: { path: string; regions: Region[] | null; names: string[] }[],
    detect: DetectOptions,
    square: SquareOptions,
    format: 'png' | 'webp',
    dir: string | null,
  ) {
    const results = await invoke<({ Ok: SplitResult } | { Err: string })[]>('export_regions', { items, detect, square, format, dir });
    const ok: SplitResult[] = [];
    const errors: string[] = [];
    for (const r of results) {
      if ('Ok' in r) ok.push(r.Ok);
      else errors.push(r.Err);
    }
    return { ok, errors };
  },
};

// 开发环境浏览器 Mock 模式下图片是 blob: URL，直接使用
export const fileSrc = (path: string) => (path.startsWith('blob:') ? path : convertFileSrc(path));

export function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(2)}MB`;
  return `${(bytes / 1024).toFixed(2)}KB`;
}

/** 体积变化百分比，正数表示减少 */
export function savedPercent(before: number, after: number): number {
  if (!before) return 0;
  return Math.round((1 - after / before) * 100);
}
