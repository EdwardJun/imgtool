import { useEffect, useState } from 'react';
import { App, Button, Empty, Spin, Tag, Tooltip } from 'antd';
import { ApartmentOutlined, DeleteOutlined, DownloadOutlined, PlusSquareOutlined, WarningOutlined } from '@ant-design/icons';
import { open } from '@tauri-apps/plugin-dialog';
import { api, fileSrc, formatBytes } from '../api';
import type { GenResult } from '../generate/useGenerator';

interface Props {
  results: GenResult[];
  onRemove: (id: string) => void;
  onClear: () => void;
  /** 把生成结果加入图片列表，继续压缩/裁剪/转格式 */
  onUse: (paths: string[]) => void;
  /** 在「设计解析」中打开 */
  onDesign: (path: string) => void;
}

function Elapsed({ from, to }: { from: number; to?: number }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (to) return;
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, [to]);
  return <span>{Math.round(((to ?? now) - from) / 1000)}s</span>;
}

export function GenerateResults({ results, onRemove, onClear, onUse, onDesign }: Props) {
  const { message } = App.useApp();
  const done = results.filter((r) => r.status === 'done' && r.file);

  const saveTo = async (paths: string[]) => {
    const dir = await open({ directory: true, multiple: false, title: '保存到目录' });
    if (typeof dir !== 'string') return;
    try {
      const saved = await api.saveFiles(paths, dir);
      message.success(`已保存 ${saved.length} 张到 ${dir}`);
    } catch (e) {
      message.error(String(e));
    }
  };

  if (!results.length) {
    return (
      <div className="gen-empty">
        <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="在右侧填写提示词并点击「生成图片」" />
      </div>
    );
  }

  return (
    <div className="gen-results">
      <div className="batch-bar">
        <span>
          共 {results.length} 个任务，完成 {done.length} 张
        </span>
        <span className="gen-bar-actions">
          <Button size="small" icon={<PlusSquareOutlined />} disabled={!done.length} onClick={() => onUse(done.map((r) => r.file!.path))}>
            全部加入图片列表
          </Button>
          <Button size="small" icon={<DownloadOutlined />} disabled={!done.length} onClick={() => saveTo(done.map((r) => r.file!.path))}>
            全部保存
          </Button>
          <Button size="small" onClick={onClear}>
            清空结果
          </Button>
        </span>
      </div>
      <div className="gen-grid">
        {results.map((r) => (
          <div key={r.id} className={`gen-card ${r.status}`}>
            <div className="gen-thumb checkerboard">
              {r.status === 'running' && (
                <div className="gen-wait">
                  <Spin />
                  <span>
                    生成中 <Elapsed from={r.startedAt} />
                  </span>
                </div>
              )}
              {r.status === 'failed' && <div className="gen-error">{r.error}</div>}
              {r.status === 'done' && r.file && <img src={fileSrc(r.file.path)} alt={r.prompt} />}
            </div>
            <div className="gen-meta">
              <Tooltip title={r.file?.revised_prompt || r.prompt}>
                <div className="gen-prompt">{r.prompt}</div>
              </Tooltip>
              <div className="muted gen-sub">
                {r.file && r.file.warnings.length > 0 && (
                  <Tooltip title={r.file.warnings.join('\n')}>
                    <WarningOutlined className="warn-icon" aria-label={r.file.warnings.join('；')} />{' '}
                  </Tooltip>
                )}
                <Tag>{r.model}</Tag>
                {r.file ? `${r.file.width}×${r.file.height} · ${r.file.format} · ${formatBytes(r.file.bytes)}` : r.size}
                {r.finishedAt && (
                  <>
                    {' · '}
                    <Elapsed from={r.startedAt} to={r.finishedAt} />
                  </>
                )}
              </div>
              <div className="gen-actions">
                {r.file && (
                  <>
                    <Button size="small" type="text" icon={<PlusSquareOutlined />} onClick={() => onUse([r.file!.path])}>
                      加入列表
                    </Button>
                    <Button size="small" type="text" icon={<DownloadOutlined />} onClick={() => saveTo([r.file!.path])}>
                      保存
                    </Button>
                    <Tooltip title={r.file.spec_path ? '已解析，打开查看与标注' : '解析成设计稿结构化描述'}>
                      <Button size="small" type="text" icon={<ApartmentOutlined />} onClick={() => onDesign(r.file!.path)}>
                        设计解析
                      </Button>
                    </Tooltip>
                  </>
                )}
                {r.status !== 'running' && <Button size="small" type="text" icon={<DeleteOutlined />} aria-label="删除结果" onClick={() => onRemove(r.id)} />}
              </div>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
