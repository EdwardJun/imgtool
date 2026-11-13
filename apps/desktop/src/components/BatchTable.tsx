import { Button, Checkbox, Table, Tag, Tooltip } from 'antd';
import { DeleteOutlined } from '@ant-design/icons';
import { fileSrc, formatBytes, savedPercent } from '../api';
import type { ImageItem } from '../types';

interface Props {
  items: ImageItem[];
  estimating: boolean;
  onToggle: (i: number) => void;
  onToggleAll: (v: boolean) => void;
  onRemove: (i: number) => void;
  onOpen: (i: number) => void;
  onEstimate: () => void;
}

export function BatchTable({ items, estimating, onToggle, onToggleAll, onRemove, onOpen, onEstimate }: Props) {
  const allSelected = items.length > 0 && items.every((i) => i.selected);
  const someSelected = items.some((i) => i.selected);

  return (
    <div className="batch">
      <div className="batch-bar">
        <span>
          共 {items.length} 张，已选 {items.filter((i) => i.selected).length} 张
        </span>
        <Button size="small" loading={estimating} onClick={onEstimate} disabled={!items.length}>
          计算全部预估
        </Button>
      </div>
      <Table<ImageItem>
        size="small"
        rowKey="path"
        pagination={false}
        dataSource={items}
        scroll={{ y: 'calc(100vh - 220px)' }}
        columns={[
          {
            title: <Checkbox aria-label="全选" checked={allSelected} indeterminate={!allSelected && someSelected} onChange={(e) => onToggleAll(e.target.checked)} />,
            width: 44,
            render: (_, it, i) => <Checkbox aria-label={`选择 ${it.name}`} checked={it.selected} onChange={() => onToggle(i)} />,
          },
          {
            title: '图片',
            render: (_, it, i) => (
              <button className="batch-name" onClick={() => onOpen(i)}>
                <img src={fileSrc(it.display_path)} alt="" loading="lazy" />
                <span>{it.name}</span>
              </button>
            ),
          },
          {
            title: '原图',
            width: 150,
            render: (_, it) => (
              <span className="muted">
                {formatBytes(it.info.bytes)} · {it.info.width}×{it.info.height}
              </span>
            ),
          },
          {
            title: '处理后',
            width: 200,
            render: (_, it) => {
              const r = it.exportState?.report ?? it.preview?.report;
              if (it.previewError) return <Tag color="error">{it.previewError}</Tag>;
              if (!r) return <span className="muted">—</span>;
              const pct = savedPercent(r.original_bytes, r.output_bytes);
              return (
                <span>
                  {formatBytes(r.output_bytes)} · {r.width}×{r.height} <Tag color={pct > 0 ? 'success' : 'default'}>↓{Math.max(0, pct)}%</Tag>
                </span>
              );
            },
          },
          {
            title: '状态',
            width: 110,
            render: (_, it) => {
              const s = it.exportState;
              if (!s) return <span className="muted">待导出</span>;
              if (!s.ok) return <Tooltip title={s.error}><Tag color="error">失败</Tag></Tooltip>;
              return s.output ? <Tooltip title={s.output}><Tag color="success">已导出</Tag></Tooltip> : <Tooltip title="压缩后没有变小，保留原图"><Tag>无需导出</Tag></Tooltip>;
            },
          },
          {
            width: 44,
            render: (_, it, i) => <Button type="text" size="small" icon={<DeleteOutlined />} aria-label={`移除 ${it.name}`} onClick={() => onRemove(i)} />,
          },
        ]}
      />
    </div>
  );
}
