// hfsyapi 图片模型能力表（来自 https://www.hfsyapi.cn/docs/api 各模型文档，2026-11 读取）
// 所有模型共用同一套接口：POST /v1/image/create + GET /v1/image/query/{id}，只换 model 名称

export type Tier = '1K' | '2K' | '4K';
export const TIERS: Tier[] = ['1K', '2K', '4K'];

/** 比例 → [1K, 2K, 4K] 像素尺寸 */
export const SIZE_TABLE: Record<string, [string, string, string]> = {
  '1:1': ['1024x1024', '2048x2048', '2880x2880'],
  '5:4': ['1040x832', '2080x1664', '3200x2560'],
  '4:3': ['1024x768', '2048x1536', '3264x2448'],
  '3:2': ['1008x672', '2016x1344', '3504x2336'],
  '16:9': ['1280x720', '2048x1152', '3840x2160'],
  '21:9': ['1344x576', '2016x864', '3696x1584'],
  '3:1': ['1008x336', '2016x672', '3504x1168'],
  '4:5': ['832x1040', '1664x2080', '2560x3200'],
  '3:4': ['768x1024', '1536x2048', '2448x3264'],
  '2:3': ['672x1008', '1344x2016', '2336x3504'],
  '9:16': ['720x1280', '1152x2048', '2160x3840'],
};

const RATIOS_STD = ['1:1', '5:4', '4:3', '3:2', '16:9', '21:9', '4:5', '3:4', '2:3', '9:16'];
const RATIOS_FLARE = ['1:1', '5:4', '4:3', '3:2', '16:9', '3:1', '3:4', '9:16'];

export interface ModelSpec {
  id: string;
  group: string;
  tiers: Tier[];
  ratios: string[];
  maxRefs: number;
  /** 支持 background: transparent */
  transparent: boolean;
}

const m = (id: string, group: string, tiers: Tier[], ratios: string[], maxRefs: number, transparent = false): ModelSpec => ({
  id,
  group,
  tiers,
  ratios,
  maxRefs,
  transparent,
});

// 文档写 flare/sunburst 支持 background: transparent，但 2026-11-13 实测服务端返回
// 「不支持参数: background」，所以全部标为不支持；服务端恢复后把最后一个参数改回 true
export const MODELS: ModelSpec[] = [
  m('gpt-image-2', 'OpenAI', ['1K'], RATIOS_STD, 6),
  m('gpt-image-2.5', 'OpenAI', ['1K'], RATIOS_STD, 6),
  m('gpt-image-2.5-flare', 'OpenAI', TIERS, RATIOS_FLARE, 16),
  m('gpt-image-2.5-sunburst', 'OpenAI', TIERS, RATIOS_FLARE, 16),
  m('gpt-image-2.6', 'OpenAI', ['1K'], RATIOS_STD, 6),
  m('gpt-image-2.6-flare', 'OpenAI', TIERS, RATIOS_FLARE, 16),
  m('gpt-image-2.6-sunburst', 'OpenAI', TIERS, RATIOS_FLARE, 16),
  m('gpt-image-2pro', 'OpenAI', ['2K'], RATIOS_STD, 4),
  m('gpt-image-2pro-4k', 'OpenAI', ['2K', '4K'], RATIOS_STD, 4),
  m('nano-banana-2', 'Google', TIERS, RATIOS_STD, 4),
  m('nano-banana-2.1', 'Google', TIERS, RATIOS_STD, 4),
  m('nano-banana-pro', 'Google', TIERS, RATIOS_STD, 7),
  m('gemini-3-pro-image-preview', 'Google', TIERS, RATIOS_STD, 7),
  m('gemini-3.1-flash-image-preview', 'Google', TIERS, RATIOS_STD, 4),
];

/** 自定义模型名：不限制比例和档位，由服务端校验 */
export function specFor(model: string): ModelSpec {
  return (
    MODELS.find((x) => x.id === model.trim()) ??
    m(model.trim(), '自定义', TIERS, Object.keys(SIZE_TABLE), 16, false)
  );
}

export function sizeFor(ratio: string, tier: Tier): { width: number; height: number } {
  const row = SIZE_TABLE[ratio] ?? SIZE_TABLE['1:1'];
  const [w, h] = row[TIERS.indexOf(tier)].split('x').map(Number);
  return { width: w, height: h };
}

/**
 * 为最终尺寸挑选生成规格：比例取模型支持的比例里最接近目标宽高比的（按对数距离），
 * 档位取能覆盖目标长边的最低档，避免先生成小图再放大；都不够时用最高档。
 */
export function matchRatioTier(spec: ModelSpec, width: number, height: number): { ratio: string; tier: Tier } {
  const target = Math.log(width / height);
  const ratio = spec.ratios.reduce((best, r) => {
    const [a, b] = r.split(':').map(Number);
    const [ba, bb] = best.split(':').map(Number);
    return Math.abs(Math.log(a / b) - target) < Math.abs(Math.log(ba / bb) - target) ? r : best;
  }, spec.ratios[0]);
  const need = Math.max(width, height);
  const tier = spec.tiers.find((t) => {
    const s = sizeFor(ratio, t);
    return Math.max(s.width, s.height) >= need;
  }) ?? spec.tiers[spec.tiers.length - 1];
  return { ratio, tier };
}

export const DEFAULT_BASE_URL = 'https://www.hfsyapi.cn';

/** hfsyapi 只接受 http(s) 图片地址作为参考图：data URL 会被拒（请求体 ≤1MB、URL 长度受限），也没有上传接口 */
export const isHfsyapi = (baseUrl: string) => /hfsyapi\.cn/i.test(baseUrl);
export const PROMPT_MAX = 5000;
