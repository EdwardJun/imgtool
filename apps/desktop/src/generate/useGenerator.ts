import { useCallback, useEffect, useRef, useState } from 'react';
import { api } from '../api';
import type { GenerateParams, GeneratedFile } from '../types';
import { DEFAULT_BASE_URL, type Tier } from './models';

export interface GenResult {
  id: string;
  prompt: string;
  model: string;
  size: string;
  startedAt: number;
  finishedAt?: number;
  status: 'running' | 'done' | 'failed';
  file?: GeneratedFile;
  error?: string;
}

export interface GenForm {
  model: string;
  ratio: string;
  tier: Tier;
  format: 'png' | 'jpeg' | 'webp' | 'avif';
  count: number;
  transparent: boolean;
  /** 生成后本地去除背景 */
  removeBg: boolean;
  /** 生成后本地调整到的最终尺寸 */
  finalSize: { enabled: boolean; width: number | null; height: number | null; fit: 'cover' | 'contain' | 'fill' };
  /** 生成后解析成设计稿 */
  analyzeDesign: boolean;
}

const FORM_KEY = 'imgtool.generate.form';
export const defaultForm: GenForm = {
  model: 'gpt-image-2',
  ratio: '1:1',
  tier: '1K',
  format: 'png',
  count: 1,
  transparent: false,
  removeBg: false,
  finalSize: { enabled: false, width: 1024, height: 1024, fit: 'cover' },
  analyzeDesign: false,
};

/** 非敏感的生成参数记在 localStorage */
export function loadForm(): GenForm {
  try {
    const saved = JSON.parse(localStorage.getItem(FORM_KEY) ?? '{}');
    return { ...defaultForm, ...saved, finalSize: { ...defaultForm.finalSize, ...saved.finalSize } };
  } catch {
    return defaultForm;
  }
}
export function saveForm(f: GenForm) {
  localStorage.setItem(FORM_KEY, JSON.stringify(f));
}

export interface ApiForm {
  base_url: string;
  api_key: string;
  remember: boolean;
  /** 提示词优化用的对话模型，为空时用默认模型 */
  prompt_model: string;
}

export const DEFAULT_PROMPT_MODEL = 'deepseek-v4.1-flash';

/** 接口地址和 API Key 保存在本机应用配置目录（由 Rust 侧读写） */
export function useApiSettings() {
  const [value, setValue] = useState<ApiForm>({ base_url: DEFAULT_BASE_URL, api_key: '', remember: true, prompt_model: DEFAULT_PROMPT_MODEL });
  const [loaded, setLoaded] = useState(false);
  useEffect(() => {
    api
      .getApiSettings()
      .then((s) => {
        if (s.base_url || s.api_key)
          setValue({ base_url: s.base_url || DEFAULT_BASE_URL, api_key: s.api_key, remember: true, prompt_model: s.prompt_model || DEFAULT_PROMPT_MODEL });
      })
      .catch(() => undefined)
      .finally(() => setLoaded(true));
  }, []);
  const save = useCallback(async (v: ApiForm) => {
    // 先写盘再更新界面，写入失败时保留原值
    await api.saveApiSettings(v);
    setValue(v);
  }, []);
  return { value, setValue, save, loaded };
}

let seq = 0;

export function useGenerator() {
  const [results, setResults] = useState<GenResult[]>([]);
  const mounted = useRef(true);
  useEffect(() => {
    // StrictMode 下会先卸载再挂载，必须在挂载时重新置为 true
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const patch = (id: string, p: Partial<GenResult>) => {
    if (mounted.current) setResults((prev) => prev.map((r) => (r.id === id ? { ...r, ...p } : r)));
  };

  /** 每张图一个任务，并发执行；结果逐个回填 */
  const generate = useCallback(async (params: GenerateParams, count: number) => {
    const batch: GenResult[] = Array.from({ length: count }, () => ({
      id: `g${++seq}`,
      prompt: params.prompt,
      model: params.model,
      size: `${params.width}x${params.height}`,
      startedAt: Date.now(),
      status: 'running',
    }));
    setResults((prev) => [...batch, ...prev]);
    await Promise.all(
      batch.map(async (r) => {
        try {
          const file = await api.generateImage(params);
          patch(r.id, { status: 'done', file, finishedAt: Date.now() });
        } catch (e) {
          patch(r.id, { status: 'failed', error: String(e), finishedAt: Date.now() });
        }
      }),
    );
  }, []);

  const remove = (id: string) => setResults((prev) => prev.filter((r) => r.id !== id));
  const clear = () => setResults((prev) => prev.filter((r) => r.status === 'running'));
  const running = results.some((r) => r.status === 'running');

  return { results, generate, remove, clear, running };
}
