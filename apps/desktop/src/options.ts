import type {
  AspectPreset,
  BackgroundRemoval,
  DetectOptions,
  ImageItem,
  PhotoSizeKey,
  ProcessOptions,
  Rect,
  RemoveBgSettings,
  ResizeSpec,
  Settings,
  SmartCropSettings,
  Tool,
} from './types';

export const defaultRemoveBg: RemoveBgSettings = {
  auto: true,
  color: '#ffffff',
  tolerance: 12,
  softness: 8,
  contiguous: true,
  decontaminate: true,
  trim: false,
  trimPadding: 0,
  format: 'png',
  mode: 'transparent',
  fill: '#438edb',
  portrait: true,
  method: 'color',
  // 更换底色大多是人像，默认用 AI 抠图
  fillMethod: 'ai',
  photoSize: 'original',
  customWidth: null,
  customHeight: null,
  limitSize: false,
  maxKb: 50,
  fillFormat: 'jpeg',
};

/** 证件照尺寸（与 imgcore::portrait::PHOTO_SIZES 保持一致） */
export const PHOTO_SIZES: { key: Exclude<PhotoSizeKey, 'original' | 'custom'>; label: string; width: number; height: number }[] = [
  { key: '1inch', label: '一寸', width: 295, height: 413 },
  { key: '2inch', label: '二寸', width: 413, height: 579 },
  { key: 'small-1inch', label: '小一寸', width: 260, height: 378 },
  { key: 'small-2inch', label: '小二寸', width: 413, height: 531 },
];

/** 常用证件照底色（与 imgcore::portrait::FILL_COLORS 保持一致） */
export const FILL_PRESETS: { label: string; color: string }[] = [
  { label: '白', color: '#ffffff' },
  { label: '蓝', color: '#438edb' },
  { label: '红', color: '#ff0000' },
];

/** 当前选择的证件照输出尺寸；原尺寸或自定义未填完整时为 null */
export function photoSizeOf(r: RemoveBgSettings): { width: number; height: number } | null {
  if (r.photoSize === 'custom') return r.customWidth && r.customHeight ? { width: r.customWidth, height: r.customHeight } : null;
  return PHOTO_SIZES.find((p) => p.key === r.photoSize) ?? null;
}

export const frameKeyOf = (size: { width: number; height: number }) => `${size.width}x${size.height}`;

const REMOVE_BG_KEY = 'imgtool.removeBg';

export function loadRemoveBg(): RemoveBgSettings {
  try {
    return { ...defaultRemoveBg, ...JSON.parse(localStorage.getItem(REMOVE_BG_KEY) ?? '{}') };
  } catch {
    return defaultRemoveBg;
  }
}

export function saveRemoveBg(s: RemoveBgSettings) {
  localStorage.setItem(REMOVE_BG_KEY, JSON.stringify(s));
}

export function hexToRgb(hex: string): [number, number, number] {
  const h = hex.replace('#', '');
  const v = (i: number) => parseInt(h.slice(i, i + 2), 16) || 0;
  return [v(0), v(2), v(4)];
}

export function rgbToHex([r, g, b]: [number, number, number]): string {
  return `#${[r, g, b].map((v) => v.toString(16).padStart(2, '0')).join('')}`;
}

/** 界面设置 → Rust 侧 BackgroundRemoval */
export function toRemoval(s: RemoveBgSettings): BackgroundRemoval {
  return {
    color: s.auto ? null : hexToRgb(s.color),
    tolerance: s.tolerance,
    softness: s.softness,
    contiguous: s.contiguous,
    decontaminate: s.decontaminate,
    trim: s.trim,
    trim_padding: s.trim ? s.trimPadding : 0,
    method: s.method,
  };
}

export const QUALITY_NORMAL = 85;
export const QUALITY_HIGH = 70;

export const defaultSettings: Settings = {
  compress: {
    mode: 'normal',
    targetValue: 200,
    targetUnit: 'KB',
    quality: 80,
    resize: { mode: 'original', percent: 50, width: null, height: null, lock: true },
    format: 'original',
  },
  crop: { mode: 'manual', aspect: 'free', outWidth: null, outHeight: null, quality: 90 },
  convert: { format: 'webp', quality: 80, lossless: false },
  removeBg: defaultRemoveBg,
  // 默认去掉文字，只保留图标
  smartCrop: { includeLabels: false, gap: null, minSize: null, size: 256, padding: 10, format: 'png' },
  output: { mode: 'suffix', dir: null, suffix: '.min' },
};

// v2：默认值改为只保留图标；换 key 让旧版本保存的"包含文字"不再覆盖新默认值
const SMART_KEY = 'imgtool.smartCrop.v2';

export function loadSmartCrop(): SmartCropSettings {
  try {
    return { ...defaultSettings.smartCrop, ...JSON.parse(localStorage.getItem(SMART_KEY) ?? '{}') };
  } catch {
    return defaultSettings.smartCrop;
  }
}

export function saveSmartCrop(s: SmartCropSettings) {
  localStorage.setItem(SMART_KEY, JSON.stringify(s));
}

export function toDetectOptions(s: SmartCropSettings): DetectOptions {
  return { include_labels: s.includeLabels, gap: s.gap, min_size: s.minSize, tolerance: 12 };
}

/** 识别参数快照，参数不变就不重复识别 */
export const detectKey = (s: SmartCropSettings) => JSON.stringify(toDetectOptions(s));

export function aspectRatio(aspect: AspectPreset, item: ImageItem): number | null {
  switch (aspect) {
    case 'free':
      return null;
    case 'original':
      return (item.info.width ?? 1) / (item.info.height ?? 1);
    default: {
      const [w, h] = aspect.split(':').map(Number);
      return w / h;
    }
  }
}

/** 按宽高比生成居中的最大裁剪框 */
export function defaultCropRect(item: ImageItem, ratio: number | null): Rect {
  const W = item.info.width ?? 0;
  const H = item.info.height ?? 0;
  if (!ratio) return { x: 0, y: 0, width: W, height: H };
  let width = W;
  let height = Math.round(W / ratio);
  if (height > H) {
    height = H;
    width = Math.round(H * ratio);
  }
  return { x: Math.round((W - width) / 2), y: Math.round((H - height) / 2), width, height };
}

/** 压缩页的分辨率设置换算成每张图的缩放参数 */
function compressResize(settings: Settings, item: ImageItem): ResizeSpec | null {
  const r = settings.compress.resize;
  const W = item.info.width ?? 0;
  switch (r.mode) {
    case 'original':
      return null;
    case 'percent':
      if (r.percent >= 100) return null;
      return { width: Math.max(1, Math.round((W * r.percent) / 100)), height: null, fit: 'contain', without_enlargement: true };
    case 'max':
      if (!r.width && !r.height) return null;
      return { width: r.width, height: r.height, fit: 'contain', without_enlargement: true };
    case 'custom':
      if (!r.width && !r.height) return null;
      // 锁定比例时只传宽（或高），另一边按原图比例计算
      if (r.lock) return { width: r.width ?? null, height: r.width ? null : r.height, fit: 'contain', without_enlargement: false };
      return { width: r.width, height: r.height, fit: r.width && r.height ? 'fill' : 'contain', without_enlargement: false };
  }
}

export function buildOptions(tool: Tool, settings: Settings, item: ImageItem): ProcessOptions {
  if (tool === 'removebg' && settings.removeBg.mode === 'replace') {
    const r = settings.removeBg;
    const size = photoSizeOf(r);
    const frame = size && item.frame && item.frameKey === frameKeyOf(size) ? item.frame : null;
    return {
      // 证件照按主体构图，不需要先裁掉透明空白
      remove_background: { ...toRemoval(r), trim: false, trim_padding: 0, fill: hexToRgb(r.fill), portrait: r.portrait, method: r.fillMethod },
      crop: size ? (frame ? { type: 'rect', ...frame } : { type: 'subject', width: size.width, height: size.height }) : null,
      resize: size ? { width: size.width, height: size.height, fit: 'cover', without_enlargement: false } : null,
      format: r.fillFormat,
      // 限制体积时只降质量、不缩小分辨率，保证证件照像素尺寸
      mode: r.limitSize
        ? { type: 'target_size', max_bytes: Math.max(1, Math.round(r.maxKb * 1024)), keep_dimensions: true }
        : r.fillFormat === 'png'
          ? { type: 'lossless' }
          : { type: 'quality', quality: 92 },
      keep_original_if_larger: false,
    };
  }
  if (tool === 'removebg') {
    const r = settings.removeBg;
    return {
      remove_background: toRemoval(r),
      format: r.format,
      // PNG 无损保留平滑边缘；WebP/AVIF 用高质量有损
      mode: r.format === 'png' ? { type: 'lossless' } : { type: 'quality', quality: 90 },
      keep_original_if_larger: false,
    };
  }
  if (tool === 'crop') {
    const c = settings.crop;
    const rect = item.crop ?? defaultCropRect(item, aspectRatio(c.aspect, item));
    const resize: ResizeSpec | null =
      c.outWidth || c.outHeight
        ? { width: c.outWidth, height: c.outHeight, fit: c.outWidth && c.outHeight ? 'cover' : 'contain', without_enlargement: false }
        : null;
    return {
      crop: { type: 'rect', ...rect },
      resize,
      format: null,
      mode: { type: 'quality', quality: c.quality },
      keep_original_if_larger: true,
    };
  }
  if (tool === 'convert') {
    const c = settings.convert;
    const lossless = c.lossless && (c.format === 'png' || c.format === 'webp');
    return {
      format: c.format,
      mode: lossless ? { type: 'lossless' } : { type: 'quality', quality: c.quality },
      keep_original_if_larger: false,
    };
  }
  const c = settings.compress;
  const mode: ProcessOptions['mode'] =
    c.mode === 'normal'
      ? { type: 'quality', quality: QUALITY_NORMAL }
      : c.mode === 'high'
        ? { type: 'quality', quality: QUALITY_HIGH }
        : c.mode === 'custom'
          ? { type: 'quality', quality: c.quality }
          : { type: 'target_size', max_bytes: Math.max(1, Math.round(c.targetValue * (c.targetUnit === 'MB' ? 1024 * 1024 : 1024))) };
  return {
    resize: compressResize(settings, item),
    format: c.format === 'original' ? null : c.format,
    mode,
    keep_original_if_larger: true,
  };
}

/** 从项目预设（.imgtool.toml）生成压缩页设置 */
export function settingsFromPreset(base: Settings, options: ProcessOptions): Settings {
  const compress = { ...base.compress, resize: { ...base.compress.resize } };
  if (options.mode.type === 'target_size') {
    compress.mode = 'target';
    compress.targetValue = Math.round(options.mode.max_bytes / 1024);
    compress.targetUnit = 'KB';
  } else if (options.mode.type === 'quality') {
    compress.mode = 'custom';
    compress.quality = options.mode.quality;
  }
  if (options.resize && (options.resize.width || options.resize.height)) {
    compress.resize = { ...compress.resize, mode: 'max', width: options.resize.width ?? null, height: options.resize.height ?? null };
  }
  compress.format = options.format ?? 'original';
  return { ...base, compress };
}
