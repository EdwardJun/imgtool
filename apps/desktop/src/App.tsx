import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Alert, App as AntApp, Button, Segmented, Spin, Tooltip } from 'antd';
import {
  ApiOutlined,
  AppstoreOutlined,
  DesktopOutlined,
  ExportOutlined,
  LeftOutlined,
  RightOutlined,
  WarningOutlined,
} from '@ant-design/icons';
import { open } from '@tauri-apps/plugin-dialog';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWebview } from '@tauri-apps/api/webview';

import { api, fileSrc, formatBytes, IMAGE_EXTENSIONS, savedPercent } from './api';
import { aspectRatio, buildOptions, defaultCropRect, defaultSettings, settingsFromPreset } from './options';
import type { ExportProgress, FoundPreset, ImageItem, Rect, Settings, Tool } from './types';
import { CompareView } from './components/CompareView';
import { CropEditor } from './components/CropEditor';
import { FitBox } from './components/FitBox';
import { Thumbnails } from './components/Thumbnails';
import { BatchTable } from './components/BatchTable';
import { McpModal } from './components/McpModal';
import { CompressPanel } from './panels/CompressPanel';
import { CropPanel } from './panels/CropPanel';
import { ConvertPanel } from './panels/ConvertPanel';
import { GeneratePanel } from './panels/GeneratePanel';
import { OutputSection } from './panels/OutputSection';

const TOOLS: { label: string; value: Tool }[] = [
  { label: '图片压缩', value: 'compress' },
  { label: '裁剪尺寸', value: 'crop' },
  { label: '格式转换', value: 'convert' },
  { label: '图片生成', value: 'generate' },
];

const previewKey = (path: string, options: unknown) => `${path}|${JSON.stringify(options)}`;

export default function App() {
  const { message } = AntApp.useApp();
  const [items, setItems] = useState<ImageItem[]>([]);
  const [current, setCurrent] = useState(0);
  const [tool, setTool] = useState<Tool>('compress');
  const [view, setView] = useState<'preview' | 'batch'>('preview');
  const [settings, setSettings] = useState<Settings>(defaultSettings);
  const [preset, setPreset] = useState<FoundPreset | null>(null);
  const [presetChecked, setPresetChecked] = useState(false);
  const [previewing, setPreviewing] = useState(false);
  const [estimating, setEstimating] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [dragOver, setDragOver] = useState(false);
  const [mcpOpen, setMcpOpen] = useState(false);
  const previewSeq = useRef(0);
  const itemsRef = useRef(items);
  itemsRef.current = items;

  const item = items[current] as ImageItem | undefined;
  const updateItem = useCallback((path: string, patch: Partial<ImageItem>) => {
    setItems((prev) => prev.map((it) => (it.path === path ? { ...it, ...patch } : it)));
  }, []);

  // ---------- 添加图片 ----------
  const addPaths = useCallback(
    async (paths: string[]) => {
      try {
        const expanded = await api.expandPaths(paths);
        const known = new Set(itemsRef.current.map((i) => i.path));
        const fresh = expanded.filter((p) => !known.has(p));
        if (!fresh.length) {
          if (!expanded.length) message.warning('没有找到支持的图片');
          return;
        }
        const { images, errors } = await api.loadImages(fresh);
        if (errors.length) message.warning(errors.slice(0, 3).join('\n') + (errors.length > 3 ? ` 等 ${errors.length} 个文件无法读取` : ''));
        if (!images.length) return;
        const wasEmpty = itemsRef.current.length === 0;
        setItems((prev) => [...prev, ...images.map((img) => ({ ...img, selected: true }))]);
        if (wasEmpty) setCurrent(0);
        if (!presetChecked) {
          setPresetChecked(true);
          api.findPreset(images[0].path).then(setPreset).catch(() => undefined);
        }
      } catch (e) {
        message.error(String(e));
      }
    },
    [message, presetChecked],
  );

  const pickFiles = async () => {
    const picked = await open({ multiple: true, title: '选择图片', filters: [{ name: '图片', extensions: IMAGE_EXTENSIONS }] });
    if (Array.isArray(picked)) addPaths(picked);
    else if (typeof picked === 'string') addPaths([picked]);
  };

  // 启动参数里带的文件直接打开
  const initialLoaded = useRef(false);
  useEffect(() => {
    if (initialLoaded.current) return;
    initialLoaded.current = true;
    api
      .initialPaths()
      .then((paths) => {
        if (paths.length) addPaths(paths);
      })
      .catch(() => undefined);
  }, [addPaths]);

  useEffect(() => {
    const unlisten = getCurrentWebview().onDragDropEvent((event) => {
      const p = event.payload;
      if (p.type === 'over' || p.type === 'enter') setDragOver(true);
      else if (p.type === 'leave') setDragOver(false);
      else if (p.type === 'drop') {
        setDragOver(false);
        addPaths(p.paths);
      }
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [addPaths]);

  const clearAll = () => {
    setItems([]);
    setCurrent(0);
    setPreset(null);
    setPresetChecked(false);
  };

  const removeAt = (i: number) => {
    setItems((prev) => prev.filter((_, idx) => idx !== i));
    setCurrent((c) => Math.max(0, Math.min(c, items.length - 2)));
  };
  // ---------- 裁剪框 ----------
  const ratio = item ? aspectRatio(settings.crop.aspect, item) : null;

  // 进入裁剪页或切换图片时，为还没有裁剪框的图片生成默认框
  useEffect(() => {
    if (tool === 'crop' && item && !item.crop) updateItem(item.path, { crop: defaultCropRect(item, ratio) });
  }, [tool, item, ratio, updateItem]);

  const setAspect = (crop: Settings['crop']) => {
    const aspectChanged = crop.aspect !== settings.crop.aspect;
    setSettings((s) => ({ ...s, crop }));
    if (aspectChanged && item) updateItem(item.path, { crop: defaultCropRect(item, aspectRatio(crop.aspect, item)) });
  };

  const resetAllCrops = () => {
    setItems((prev) => prev.map((it) => ({ ...it, crop: defaultCropRect(it, aspectRatio(settings.crop.aspect, it)) })));
  };

  // ---------- 预览 ----------
  const currentOptions = useMemo(() => (item && tool !== 'generate' ? buildOptions(tool, settings, item) : null), [item, tool, settings]);
  const currentKey = item && currentOptions ? previewKey(item.path, currentOptions) : null;

  useEffect(() => {
    if (!item || !currentOptions || !currentKey || item.previewKey === currentKey) return;
    const seq = ++previewSeq.current;
    const timer = window.setTimeout(async () => {
      setPreviewing(true);
      try {
        const res = await api.preview(item.path, currentOptions, item.preview?.output_path);
        if (seq === previewSeq.current) updateItem(item.path, { preview: res, previewKey: currentKey, previewError: undefined });
      } catch (e) {
        if (seq === previewSeq.current) updateItem(item.path, { previewError: String(e), previewKey: currentKey, preview: undefined });
      } finally {
        if (seq === previewSeq.current) setPreviewing(false);
      }
    }, 300);
    return () => window.clearTimeout(timer);
    // 只在参数或当前图片变化时重新计算（key 已包含路径和全部参数）
  }, [currentKey]);

  const estimateAll = async () => {
    if (tool === 'generate') return;
    setEstimating(true);
    const queue = [...itemsRef.current];
    const worker = async () => {
      for (let it = queue.shift(); it; it = queue.shift()) {
        const opts = buildOptions(tool, settings, it);
        const key = previewKey(it.path, opts);
        if (it.previewKey === key) continue;
        try {
          const res = await api.preview(it.path, opts, it.preview?.output_path);
          updateItem(it.path, { preview: res, previewKey: key, previewError: undefined });
        } catch (e) {
          updateItem(it.path, { previewError: String(e), previewKey: key });
        }
      }
    };
    await Promise.all([worker(), worker()]);
    setEstimating(false);
  };

  // ---------- 导出 ----------
  const exportSelected = async () => {
    const selected = items.filter((i) => i.selected);
    if (!selected.length) return message.warning('请至少选择一张图片');
    if (settings.output.mode === 'dir' && !settings.output.dir) return message.warning('请选择输出目录');
    setExporting(true);
    setItems((prev) => prev.map((it) => ({ ...it, exportState: undefined })));
    const unlisten = await listen<ExportProgress>('export-progress', (e) => {
      const p = e.payload;
      updateItem(p.path, { exportState: { ok: p.ok, output: p.output, error: p.error, report: p.report } });
    });
    try {
      const summary = await api.exportImages(
        selected.map((it) => ({ path: it.path, options: buildOptions(tool, settings, it) })),
        { mode: settings.output.mode, dir: settings.output.dir, suffix: settings.output.suffix },
      );
      const saved = savedPercent(summary.original_bytes, summary.output_bytes);
      const text = `完成 ${summary.succeeded}/${summary.total} 张，${formatBytes(summary.original_bytes)} → ${formatBytes(summary.output_bytes)}（节省 ${Math.max(0, saved)}%）`;
      if (summary.failed) message.warning(`${text}，${summary.failed} 张失败，详见批量列表`);
      else message.success(text);
    } catch (e) {
      message.error(String(e));
    } finally {
      unlisten();
      setExporting(false);
    }
  };

  const applyPreset = () => {
    if (!preset) return;
    setSettings((s) => settingsFromPreset(s, preset.options));
    setTool('compress');
    message.success('已应用项目预设');
  };
  // ---------- 渲染 ----------
  const report = item?.previewKey === currentKey ? item?.preview?.report : undefined;
  const pct = report ? savedPercent(report.original_bytes, report.output_bytes) : 0;
  const selectedCount = items.filter((i) => i.selected).length;

  const stage = () => {
    if (!item) {
      return (
        <button className="dropzone" onClick={pickFiles}>
          <AppstoreOutlined className="dropzone-icon" />
          <div className="dropzone-title">点击添加或拖入图片 / 文件夹</div>
          <div className="dropzone-desc">支持 JPG、PNG、WebP、GIF、BMP、TIFF、ICO</div>
        </button>
      );
    }
    const W = item.info.width ?? 1;
    const H = item.info.height ?? 1;
    return (
      <FitBox width={W} height={H}>
        {(size) =>
          tool === 'crop' && item.crop ? (
            <CropEditor
              src={fileSrc(item.display_path)}
              alt={item.name}
              imageWidth={W}
              imageHeight={H}
              boxWidth={size.width}
              rect={item.crop}
              ratio={ratio}
              onChange={(crop: Rect) => updateItem(item.path, { crop })}
            />
          ) : (
            <CompareView
              before={fileSrc(item.display_path)}
              after={tool !== 'generate' && item.previewKey === currentKey && item.preview ? fileSrc(item.preview.output_path) : undefined}
              alt={item.name}
            />
          )
        }
      </FitBox>
    );
  };

  return (
    <div className="app">
      <header className="titlebar">
        <div className="brand">
          <img src="/logo.svg" alt="" className="brand-logo" />
          <span>图片工具</span>
        </div>
        <nav className="tool-tabs" aria-label="功能">
          {TOOLS.map((t) => (
            <button key={t.value} className={`tool-tab ${tool === t.value ? 'active' : ''}`} aria-current={tool === t.value} onClick={() => setTool(t.value)}>
              {t.label}
            </button>
          ))}
        </nav>
        <Button icon={<ApiOutlined />} onClick={() => setMcpOpen(true)}>
          Agent 接入
        </Button>
      </header>

      <main className="workspace">
        <section className={`stage-area ${dragOver ? 'drag-over' : ''}`}>
          <Segmented
            className="view-switch"
            size="small"
            value={view}
            onChange={(v) => setView(v as 'preview' | 'batch')}
            options={[
              { label: '预览', value: 'preview', icon: <DesktopOutlined /> },
              { label: '批量', value: 'batch', icon: <AppstoreOutlined /> },
            ]}
          />
          {view === 'batch' ? (
            <BatchTable
              items={items}
              estimating={estimating}
              onToggle={(i) => updateItem(items[i].path, { selected: !items[i].selected })}
              onToggleAll={(v) => setItems((prev) => prev.map((it) => ({ ...it, selected: v })))}
              onRemove={removeAt}
              onOpen={(i) => {
                setCurrent(i);
                setView('preview');
              }}
              onEstimate={estimateAll}
            />
          ) : (
            <>
              <div className="stage">{stage()}</div>
              {item && tool !== 'generate' && (
                <div className="stats" aria-live="polite">
                  {item.previewError ? (
                    <span className="stats-error">
                      <WarningOutlined /> {item.previewError}
                    </span>
                  ) : report ? (
                    <>
                      <span className="muted">{formatBytes(report.original_bytes)}</span>
                      <span className="muted">→</span>
                      <strong>{formatBytes(report.output_bytes)}</strong>
                      <span className={`ratio ${pct > 0 ? 'good' : ''}`}>↓{Math.max(0, pct)}%</span>
                      <span className="muted">
                        {report.width}×{report.height}
                        {report.quality ? ` · Q${report.quality}` : ''}
                      </span>
                      {report.warnings.length > 0 && (
                        <Tooltip title={report.warnings.join('\n')}>
                          <WarningOutlined className="warn-icon" aria-label={report.warnings.join('；')} />
                        </Tooltip>
                      )}
                    </>
                  ) : (
                    <span className="muted">计算中…</span>
                  )}
                  {previewing && <Spin size="small" />}
                </div>
              )}
              {items.length > 0 && (
                <div className="pager">
                  <Button type="text" size="small" icon={<LeftOutlined />} aria-label="上一张" disabled={current === 0} onClick={() => setCurrent(current - 1)} />
                  <span>
                    {current + 1}/{items.length}
                  </span>
                  <Button type="text" size="small" icon={<RightOutlined />} aria-label="下一张" disabled={current >= items.length - 1} onClick={() => setCurrent(current + 1)} />
                </div>
              )}
              <Thumbnails
                items={items}
                current={current}
                onSelect={setCurrent}
                onToggle={(i) => updateItem(items[i].path, { selected: !items[i].selected })}
                onAdd={pickFiles}
                onClear={clearAll}
              />
            </>
          )}
        </section>

        <aside className="side-panel">
          <div className="side-scroll">
            {preset && tool !== 'generate' && (
              <Alert
                className="preset-alert"
                type="info"
                showIcon
                message="检测到项目预设"
                description={<span className="preset-path">{preset.path}</span>}
                action={
                  <Button size="small" onClick={applyPreset}>
                    应用
                  </Button>
                }
                closable
                onClose={() => setPreset(null)}
              />
            )}
            {tool === 'compress' && <CompressPanel value={settings.compress} current={item} onChange={(compress) => setSettings((s) => ({ ...s, compress }))} />}
            {tool === 'crop' && (
              <CropPanel
                value={settings.crop}
                current={item}
                rect={item?.crop}
                onChange={setAspect}
                onRectChange={(crop) => item && updateItem(item.path, { crop })}
                onApplyAll={resetAllCrops}
              />
            )}
            {tool === 'convert' && <ConvertPanel value={settings.convert} onChange={(convert) => setSettings((s) => ({ ...s, convert }))} />}
            {tool === 'generate' && <GeneratePanel />}
            {tool !== 'generate' && <OutputSection value={settings.output} onChange={(output) => setSettings((s) => ({ ...s, output }))} />}
          </div>
          {tool !== 'generate' && (
            <Button type="primary" size="large" block icon={<ExportOutlined />} loading={exporting} disabled={!selectedCount} onClick={exportSelected}>
              导出{selectedCount ? `（${selectedCount}）` : ''}
            </Button>
          )}
        </aside>
      </main>

      <McpModal open={mcpOpen} onClose={() => setMcpOpen(false)} />
    </div>
  );
}
