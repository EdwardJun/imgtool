// 与 Rust 侧 imgcore / commands 的 serde 结构保持一致

export type OutputFormat = 'jpeg' | 'png' | 'webp' | 'avif';
export const OUTPUT_FORMATS: OutputFormat[] = ['jpeg', 'png', 'webp', 'avif'];
export const FORMAT_LABEL: Record<OutputFormat, string> = { jpeg: 'JPG', png: 'PNG', webp: 'WebP', avif: 'AVIF' };

export type CompressMode =
  | { type: 'quality'; quality: number }
  | { type: 'target_size'; max_bytes: number; min_quality?: number | null; keep_dimensions?: boolean }
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
  | { type: 'aspect'; width: number; height: number }
  /** 按去背景后的主体自动构图（证件照） */
  | { type: 'subject'; width: number; height: number };

export interface BackgroundRemoval {
  color?: [number, number, number] | null;
  tolerance: number;
  softness: number;
  contiguous: boolean;
  decontaminate: boolean;
  trim: boolean;
  trim_padding: number;
  /** 更换底色；为空时输出透明 */
  fill?: [number, number, number] | null;
  /** 人像模式：保护被主体包住的同色区域（白衬衫等） */
  portrait?: boolean;
  /** 抠图方式：按颜色 / AI 人像（MODNet） */
  method?: RemovalMethod;
}

export type RemovalMethod = 'color' | 'ai';

export interface MattingModelStatus {
  available: boolean;
  path: string | null;
  bytes: number;
  license: string;
}

export interface SubjectBounds {
  x: number;
  y: number;
  width: number;
  height: number;
  top_center_x: number;
  head_width?: number | null;
  shoulder_y?: number | null;
}

export interface RemovalStats {
  color: [number, number, number];
  border_match: number;
  removed_ratio: number;
  subject?: SubjectBounds | null;
}

export interface ProcessOptions {
  remove_background?: BackgroundRemoval | null;
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
  background?: RemovalStats | null;
  /** 按主体自动构图时实际使用的裁剪框（原图坐标） */
  auto_crop?: Rect | null;
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

export interface ApiSettings {
  base_url: string;
  api_key: string;
  remember: boolean;
  /** 提示词优化用的对话模型 */
  prompt_model?: string;
}

export interface GenerateParams {
  base_url: string;
  api_key: string;
  model: string;
  prompt: string;
  width: number;
  height: number;
  reference_images: string[];
  transparent: boolean;
  format: OutputFormat;
  /** 生成后本地去除背景 */
  remove_background?: BackgroundRemoval | null;
  /** 生成后本地调整到的最终尺寸 */
  final_size?: { width: number; height: number; fit: ResizeFit } | null;
  /** 生成后解析成设计稿 spec */
  analyze_design?: boolean;
}

/** 提示词优化时附带的生成参数 */
export interface PromptContext {
  image_model?: string | null;
  ratio?: string | null;
  size?: string | null;
  remove_background: boolean;
  reference_count: number;
  final_size?: string | null;
}

export interface EnhanceResult {
  prompt: string;
  model: string;
  total_tokens: number | null;
}

/** 本地参考图上传到图床后的结果 */
export interface UploadedRef {
  path: string;
  url: string;
  /** 链接过期时间（Unix 秒） */
  expires_at: number;
  bytes: number;
  cached: boolean;
  compressed: boolean;
}

export interface GeneratedFile {
  task_id: string;
  path: string;
  format: string;
  width: number;
  height: number;
  bytes: number;
  source_url: string | null;
  revised_prompt: string | null;
  warnings: string[];
  /** 生成后解析出的设计稿 spec */
  spec_path?: string | null;
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

// ---------- 智能裁剪 ----------

export type RegionKind = 'icon' | 'icon_with_label';

export interface Region extends Rect {
  kind: RegionKind;
}

export interface DetectOptions {
  background?: [number, number, number] | null;
  tolerance: number;
  gap?: number | null;
  include_labels: boolean;
  min_size?: number | null;
}

export interface DetectResult {
  regions: Region[];
  gap: number;
  min_size: number;
  mode: 'alpha' | 'color';
  background: [number, number, number] | null;
  labels_found: number;
}

export interface SquareOptions {
  size: number;
  padding: number;
  background?: [number, number, number] | null;
}

export interface SplitFile {
  path: string;
  name: string;
  region: Region;
  bytes: number;
  written: boolean;
}

export interface SplitResult {
  input: string;
  out_dir: string;
  size: number;
  mode: string;
  files: SplitFile[];
}

/** 智能裁剪设置（保存在 localStorage） */
export interface SmartCropSettings {
  includeLabels: boolean;
  /** null = 自动 */
  gap: number | null;
  minSize: number | null;
  size: number;
  padding: number;
  format: 'png' | 'webp';
}

export interface ImageItem extends LoadedImage {
  selected: boolean;
  /** 裁剪框（原图像素坐标），每张图独立 */
  crop?: Rect;
  /** 证件照手动微调过的构图框（原图像素坐标）；为空时按主体自动构图 */
  frame?: Rect;
  /** 构图框对应的输出尺寸，如 "295x413"；尺寸变了框就失效 */
  frameKey?: string;
  /** 智能裁剪识别/编辑后的框 */
  regions?: Region[];
  regionNames?: string[];
  /** 识别时使用的参数快照，参数变化后需要重新识别 */
  regionsKey?: string;
  /** 用户手动改过框，重新识别前需要确认 */
  regionsEdited?: boolean;
  regionsError?: string;
  detectInfo?: Omit<DetectResult, 'regions'>;
  preview?: PreviewResult;
  previewKey?: string;
  previewError?: string;
  exportState?: { ok: boolean; output: string | null; error: string | null; report: ProcessReport | null };
}

export type Tool = 'compress' | 'crop' | 'convert' | 'removebg' | 'generate' | 'design';

export type PhotoSizeKey = 'original' | '1inch' | '2inch' | 'small-1inch' | 'small-2inch' | 'custom';

/** 背景处理页的设置（保存在 localStorage；生成图片时复用其中的去背景参数） */
export interface RemoveBgSettings {
  /** 透明背景 / 更换底色 */
  mode: 'transparent' | 'replace';
  /** true = 自动识别边缘背景色 */
  auto: boolean;
  /** 手动指定的背景色 #rrggbb */
  color: string;
  tolerance: number;
  softness: number;
  contiguous: boolean;
  decontaminate: boolean;
  trim: boolean;
  trimPadding: number;
  format: 'png' | 'webp' | 'avif';
  // ---------- 更换底色 ----------
  /** 新底色 #rrggbb */
  fill: string;
  /** 人像模式（仅更换底色、按颜色抠图时生效） */
  portrait: boolean;
  /** 透明背景的抠图方式 */
  method: RemovalMethod;
  /** 更换底色的抠图方式 */
  fillMethod: RemovalMethod;
  photoSize: PhotoSizeKey;
  customWidth: number | null;
  customHeight: number | null;
  /** 限制文件大小 */
  limitSize: boolean;
  maxKb: number;
  fillFormat: 'jpeg' | 'png';
}

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
  /** 手动裁剪单个区域 / 智能识别多个图标 */
  mode: 'manual' | 'smart';
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
  removeBg: RemoveBgSettings;
  smartCrop: SmartCropSettings;
  output: OutputSettings;
}
