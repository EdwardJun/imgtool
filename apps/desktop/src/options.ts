import type { AspectPreset, ImageItem, ProcessOptions, Rect, ResizeSpec, Settings, Tool } from './types';

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
  crop: { aspect: 'free', outWidth: null, outHeight: null, quality: 90 },
  convert: { format: 'webp', quality: 80, lossless: false },
  output: { mode: 'suffix', dir: null, suffix: '.min' },
};

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
