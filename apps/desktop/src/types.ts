// 与 Rust 侧 imgcore / commands 的 serde 结构保持一致

export type OutputFormat = 'jpeg' | 'png' | 'webp' | 'avif';
export const OUTPUT_FORMATS: OutputFormat[] = ['jpeg', 'png', 'webp', 'avif'];
export const FORMAT_LABEL: Record<OutputFormat, string> = { jpeg: 'JPG', png: 'PNG', webp: 'WebP', avif: 'AVIF' };

export type CompressMode =
  | { type: 'quality'; quality: number }
  | { type: 'target_size'; max_bytes: number; min_quality?: number | null }
  | { type: 'lossless' };

export type ResizeFit = 'contain' | 'cover' | 'fill';

export interface ResizeSpec {
  width?: number | null;
  height?: number | null;
  fit: ResizeFit;
  without_enlargement: boolean;
}

export type CropSpec =
  | { type: 'rect'; x: number; y: number; width: number; height: number }
  | { type: 'aspect'; width: number; height: number };

export interface ProcessOptions {
  crop?: CropSpec | null;
  resize?: ResizeSpec | null;
  format?: OutputFormat | null;
  mode: CompressMode;
  keep_original_if_larger: boolean;
  background?: [number, number, number] | null;
}

export interface ImageInfo {
  format: string;
  width: number | null;
  height: number | null;
  bytes: number;
  decodable: boolean;
}

export interface ProcessReport {
  format: OutputFormat;
  width: number;
  height: number;
  original_format: string;
  original_width: number;
  original_height: number;
  original_bytes: number;
  output_bytes: number;
  quality: number | null;
  kept_original: boolean;
  target_met: boolean | null;
  warnings: string[];
}

export interface LoadedImage {
  path: string;
  name: string;
  info: ImageInfo;
  display_path: string;
}

export interface PreviewResult {
  output_path: string;
  report: ProcessReport;
}

export type OutputMode = 'suffix' | 'overwrite' | 'dir';

export interface ExportTarget {
  mode: OutputMode;
  dir?: string | null;
  suffix?: string | null;
}

export interface ExportProgress {
  path: string;
  ok: boolean;
  output: string | null;
  report: ProcessReport | null;
  error: string | null;
}

export interface ExportSummary {
  total: number;
  succeeded: number;
  failed: number;
  original_bytes: number;
  output_bytes: number;
}

export interface FoundPreset {
  path: string;
  options: ProcessOptions;
}

export type McpClient = 'claude_code' | 'cursor' | 'kiro';

export interface McpClientStatus {
  id: McpClient;
  name: string;
  config_path: string;
  configured: boolean;
  up_to_date: boolean;
}

export interface McpStatus {
  command: string | null;
  error: string | null;
  clients: McpClientStatus[];
  snippet: string | null;
}

// ---------- 前端状态 ----------

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface ImageItem extends LoadedImage {
  selected: boolean;
  /** 裁剪框（原图像素坐标），每张图独立 */
  crop?: Rect;
  preview?: PreviewResult;
  previewKey?: string;
  previewError?: string;
  exportState?: { ok: boolean; output: string | null; error: string | null; report: ProcessReport | null };
}

export type Tool = 'compress' | 'crop' | 'convert' | 'generate';

export type ResizeMode = 'original' | 'percent' | 'max' | 'custom';

export interface CompressSettings {
  mode: 'normal' | 'high' | 'target' | 'custom';
  targetValue: number;
  targetUnit: 'KB' | 'MB';
  quality: number;
  resize: {
    mode: ResizeMode;
    percent: number;
    width: number | null;
    height: number | null;
    lock: boolean;
  };
  format: OutputFormat | 'original';
}

export type AspectPreset = 'free' | 'original' | '1:1' | '4:3' | '3:4' | '16:9' | '9:16';

export interface CropSettings {
  aspect: AspectPreset;
  outWidth: number | null;
  outHeight: number | null;
  quality: number;
}

export interface ConvertSettings {
  format: OutputFormat;
  quality: number;
  lossless: boolean;
}

export interface OutputSettings {
  mode: 'suffix' | 'dir';
  dir: string | null;
  suffix: string;
}

export interface Settings {
  compress: CompressSettings;
  crop: CropSettings;
  convert: ConvertSettings;
  output: OutputSettings;
}
