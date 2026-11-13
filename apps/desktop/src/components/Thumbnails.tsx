import { Button } from 'antd';
import { CheckOutlined } from '@ant-design/icons';
import { fileSrc } from '../api';
import type { ImageItem } from '../types';

interface Props {
  items: ImageItem[];
  current: number;
  onSelect: (i: number) => void;
  onToggle: (i: number) => void;
  onAdd: () => void;
  onClear: () => void;
}

export function Thumbnails({ items, current, onSelect, onToggle, onAdd, onClear }: Props) {
  return (
    <div className="thumbs">
      <div className="thumbs-list" role="listbox" aria-label="图片列表">
        {items.map((it, i) => (
          <div
            key={it.path}
            role="option"
            aria-selected={i === current}
            tabIndex={0}
            className={`thumb ${i === current ? 'active' : ''}`}
            onClick={() => onSelect(i)}
            onKeyDown={(e) => e.key === 'Enter' && onSelect(i)}
            title={it.name}
          >
            <img src={fileSrc(it.display_path)} alt={it.name} loading="lazy" draggable={false} />
            <button
              className={`thumb-check ${it.selected ? 'on' : ''}`}
              aria-label={it.selected ? '取消导出' : '加入导出'}
              aria-pressed={it.selected}
              onClick={(e) => {
                e.stopPropagation();
                onToggle(i);
              }}
            >
              {it.selected && <CheckOutlined />}
            </button>
          </div>
        ))}
      </div>
      <div className="thumbs-actions">
        <Button size="small" onClick={onAdd}>
          添加
        </Button>
        <Button size="small" onClick={onClear} disabled={!items.length}>
          清空
        </Button>
      </div>
    </div>
  );
}
