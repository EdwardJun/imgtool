import { useEffect, useState } from 'react';
import { App, Button, ColorPicker, Empty, Spin, Tag, Tooltip } from 'antd';
import { CodeOutlined, DeleteOutlined, DownloadOutlined, PlusSquareOutlined, WarningOutlined } from '@ant-design/icons';
import { open } from '@tauri-apps/plugin-dialog';
import { api, fileSrc, formatBytes } from '../api';
import type { IconSetItem, SvgBatch, SvgRenderOptions } from '../generate/svgIcons';

interface Props {
  batches: SvgBatch[];
  onRerender: (id: string, render: SvgRenderOptions) => Promise<void>;
  onRemove: (id: string) => void;
  onClear: () => void;
  onUse: (paths: string[]) => void;
}

function Elapsed({ from }: { from: number }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, []);
  return <span>{Math.round((now - from) / 1000)}s</span>;
}

const allFiles = (icons: IconSetItem[]) => icons.flatMap((i) => i.files ?? []);
/** 预览用最大的位图 */
const previewFile = (i: IconSetItem) => [...(i.files ?? [])].filter((f) => f.size).sort((a, b) => (b.size ?? 0) - (a.size ?? 0))[0];

export function SvgIconResults({ batches, onRerender, onRemove, onClear, onUse }: Props) {
  const { message } = App.useApp();
  const [busy, setBusy] = useState<string | null>(null);

  const saveTo = async (paths: string[]) => {
    const dir = await open({ directory: true, multiple: false, title: '保存到目录' });
    if (typeof dir !== 'string') return;
    try {
      const saved = await api.saveFiles(paths, dir);
      message.success(`已保存 ${saved.length} 个文件到 ${dir}`);
    } catch (e) {
      message.error(String(e));
    }
  };

  const copySvg = async (svg: string) => {
    try {
      await navigator.clipboard.writeText(svg);
      message.success('已复制 SVG 代码');
    } catch (e) {
      message.error(`复制失败：${e}`);
    }
  };

  const recolor = async (b: SvgBatch, color: string) => {
    if (color === b.render.color) return;
    setBusy(b.id);
    try {
      await onRerender(b.id, { ...b.render, color });
    } catch (e) {
      message.error(String(e));
    } finally {
      setBusy(null);
    }
  };

  if (!batches.length) {
    return (
      <div className="gen-empty">
        <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="在右侧填写图标清单并点击「生成图标」" />
      </div>
    );
  }

  return (
    <div className="gen-results">
      <div className="batch-bar">
        <span>共 {batches.length} 套图标</span>
        <span className="gen-bar-actions">
          <Button size="small" onClick={onClear}>
            清空结果
          </Button>
        </span>
      </div>
      <div className="svg-batches">
        {batches.map((b) => {
          const icons = b.set?.icons ?? [];
          const ok = icons.filter((i) => i.files?.length);
          const failed = [...icons.filter((i) => i.error).map((i) => `${i.label}：${i.error}`), ...(b.set?.missing ?? []).map((m) => `${m}：模型没有返回`)];
          const rasters = allFiles(ok).filter((f) => f.format !== 'svg');
          return (
            <section key={b.id} className={`svg-batch ${b.status}`}>
              <div className="svg-batch-head">
                <span className="svg-batch-title">
                  {b.labels.join('、')}
                  {b.set?.model && <Tag>{b.set.model}</Tag>}
                  {b.status === 'done' && b.finishedAt && <span className="muted">{((b.finishedAt - b.startedAt) / 1000).toFixed(1)}s</span>}
                </span>
                {b.status === 'done' && ok.length > 0 && (
                  <span className="gen-bar-actions">
                    <Tooltip title="换颜色后重新导出，不会重新调用模型">
                      <ColorPicker size="small" value={b.render.color} disabledAlpha showText disabled={busy === b.id} onChangeComplete={(c) => recolor(b, c.toHexString().toUpperCase())} />
                    </Tooltip>
                    <Button size="small" icon={<PlusSquareOutlined />} onClick={() => onUse(rasters.map((f) => f.path))}>
                      加入图片列表
                    </Button>
                    <Button size="small" type="primary" icon={<DownloadOutlined />} onClick={() => saveTo(allFiles(ok).map((f) => f.path))}>
                      全部保存
                    </Button>
                  </span>
                )}
                {b.status !== 'running' && <Button size="small" type="text" icon={<DeleteOutlined />} aria-label="删除这套图标" onClick={() => onRemove(b.id)} />}
              </div>
              {b.status === 'running' && (
                <div className="gen-wait">
                  <Spin />
                  <span>
                    正在生成 {b.labels.length} 个图标 <Elapsed from={b.startedAt} />
                  </span>
                </div>
              )}
              {b.status === 'failed' && <div className="gen-error">{b.error}</div>}
              {failed.length > 0 && (
                <div className="panel-hint warn block">
                  <WarningOutlined /> 失败 {failed.length} 个：{failed.join('；')}
                </div>
              )}
              {ok.length > 0 && (
                <Spin spinning={busy === b.id}>
                  <div className="svg-grid">
                    {ok.map((i) => {
                      const pf = previewFile(i);
                      return (
                        <div key={i.name} className="svg-card">
                          <div className="svg-thumb">{pf && <img src={fileSrc(pf.path)} alt={i.label} />}</div>
                          <div className="svg-name" title={i.name}>
                            {i.label}
                            {i.label !== i.name && <span className="muted"> · {i.name}</span>}
                            {i.warnings?.length ? (
                              <Tooltip title={i.warnings.join('\n')}>
                                <WarningOutlined className="warn-icon" aria-label={i.warnings.join('；')} />
                              </Tooltip>
                            ) : null}
                          </div>
                          <div className="muted svg-files">
                            {(i.files ?? []).map((f) => (f.size ? `${f.size}px ${formatBytes(f.bytes)}` : `svg ${formatBytes(f.bytes)}`)).join(' · ')}
                          </div>
                          <div className="gen-actions">
                            {i.svg && (
                              <Button size="small" type="text" icon={<CodeOutlined />} onClick={() => copySvg(i.svg!)}>
                                复制 SVG
                              </Button>
                            )}
                            <Button size="small" type="text" icon={<DownloadOutlined />} onClick={() => saveTo((i.files ?? []).map((f) => f.path))}>
                              保存
                            </Button>
                          </div>
                        </div>
                      );
                    })}
                  </div>
                </Spin>
              )}
            </section>
          );
        })}
      </div>
    </div>
  );
}
