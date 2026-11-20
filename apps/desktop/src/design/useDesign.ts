import { useCallback, useRef, useState } from 'react';
import { api } from '../api';
import type { Bounds, DesignResult, DesignSettings, RunOptions, Spec } from './types';
import { defaultDesignSettings } from './types';
import { findNode, mergeAnalyzed, updateNode } from './tree';

export interface DesignState {
  loaded?: boolean;
  busy?: 'analyze' | 'save' | 'normalize' | 'icons' | null;
  /** 最近一次从后端拿到的结果（spec 文件、标注图、根目录） */
  result?: DesignResult;
  /** 界面上正在编辑的 spec */
  spec?: Spec;
  dirty?: boolean;
  error?: string;
  /** 每次从后端拿到新结果加一，用于刷新图标缩略图（文件名不变、内容变了） */
  rev?: number;
}

const KEY = 'imgtool.design';

export function loadDesignSettings(): DesignSettings {
  try {
    return { ...defaultDesignSettings, ...JSON.parse(localStorage.getItem(KEY) ?? '{}') };
  } catch {
    return defaultDesignSettings;
  }
}

export function saveDesignSettings(s: DesignSettings) {
  localStorage.setItem(KEY, JSON.stringify(s));
}

export function toRunOptions(s: DesignSettings): RunOptions {
  return {
    analyze: { mode: s.mode === 'auto' ? null : s.mode, tolerance: s.tolerance, max_depth: 6, max_edge: 2560 },
    keep_annotations: true,
    export_icons: s.exportIcons,
    icons: { size: 256, padding: 10 },
  };
}

/** 每张图一份设计解析状态 */
export function useDesign() {
  const [states, setStates] = useState<Record<string, DesignState>>({});
  const ref = useRef(states);
  ref.current = states;
  const patch = useCallback((path: string, p: Partial<DesignState> | ((s: DesignState) => Partial<DesignState>)) => {
    setStates((all) => {
      const cur = all[path] ?? {};
      const next = typeof p === 'function' ? p(cur) : p;
      return { ...all, [path]: { ...cur, ...next } };
    });
  }, []);

  const apply = (path: string, r: DesignResult) =>
    patch(path, (s) => ({ result: r, spec: r.spec, dirty: false, busy: null, error: undefined, loaded: true, rev: (s.rev ?? 0) + 1 }));

  const run = async (path: string, busy: DesignState['busy'], f: () => Promise<void>) => {
    patch(path, { busy, error: undefined });
    try {
      await f();
    } catch (e) {
      patch(path, { busy: null, error: String(e) });
      throw e;
    }
  };

  /** 读取已有解析结果（只读一次） */
  const load = useCallback(
    async (path: string) => {
      if (ref.current[path]?.loaded) return;
      patch(path, { loaded: true });
      try {
        const r = await api.loadDesignSpec(path);
        if (r) apply(path, r);
      } catch (e) {
        patch(path, { error: String(e) });
      }
    },
    [patch],
  );

  const analyze = (path: string, settings: DesignSettings) =>
    run(path, 'analyze', async () => {
      apply(path, await api.analyzeDesign(path, toRunOptions(settings)));
    });

  const edit = (path: string, f: (spec: Spec) => Spec) =>
    patch(path, (s) => (s.spec ? { spec: f(s.spec), dirty: true } : {}));

  const save = (path: string) =>
    run(path, 'save', async () => {
      const spec = ref.current[path]?.spec;
      if (!spec) return;
      apply(path, await api.saveDesignSpec(path, spec));
    });

  const normalize = (path: string) =>
    run(path, 'normalize', async () => {
      const spec = ref.current[path]?.spec;
      if (!spec) return;
      const next = await api.normalizeSpec(spec);
      patch(path, { spec: next, dirty: true, busy: null });
    });

  /** 框改动后重新取样式 */
  const reanalyze = async (path: string, id: string, bounds: Bounds) => {
    try {
      const fresh = await api.reanalyzeRegion(path, bounds);
      edit(path, (spec) => ({ ...spec, root: updateNode(spec.root, id, (n) => mergeAnalyzed({ ...n, bounds }, fresh)) }));
    } catch {
      // 取样式失败不影响框本身的修改
    }
  };

  const generateIcons = async (path: string, params: { base_url: string; api_key: string; model: string; style: string; ids?: string[] | null }) => {
    let out: DesignResult | undefined;
    await run(path, 'icons', async () => {
      // 未保存的修改先落盘，AI 重绘读的是磁盘上的 spec
      const cur = ref.current[path];
      if (cur?.dirty && cur.spec) apply(path, await api.saveDesignSpec(path, cur.spec));
      out = await api.generateDesignIcons({ path, ...params, style: params.style || null });
      apply(path, out);
    });
    return out;
  };

  const node = (path: string, id: string | null) => {
    const spec = states[path]?.spec;
    return spec && id ? findNode(spec.root, id) : null;
  };

  return { states, load, analyze, edit, save, normalize, reanalyze, generateIcons, node };
}
