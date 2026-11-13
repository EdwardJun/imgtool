import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import type {
  ExportSummary,
  ExportTarget,
  FoundPreset,
  LoadedImage,
  McpClient,
  McpStatus,
  PreviewResult,
  ProcessOptions,
} from './types';

export const IMAGE_EXTENSIONS = ['jpg', 'jpeg', 'png', 'webp', 'gif', 'bmp', 'tif', 'tiff', 'ico'];

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
