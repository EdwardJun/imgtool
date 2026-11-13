import { Button, Input, Radio } from 'antd';
import { FolderOpenOutlined } from '@ant-design/icons';
import { open } from '@tauri-apps/plugin-dialog';
import type { OutputSettings } from '../types';

interface Props {
  value: OutputSettings;
  onChange: (v: OutputSettings) => void;
}

export function OutputSection({ value, onChange }: Props) {
  const pickDir = async () => {
    const dir = await open({ directory: true, multiple: false, title: '选择输出目录' });
    if (typeof dir === 'string') onChange({ ...value, mode: 'dir', dir });
  };

  return (
    <div className="panel-section output-section">
      <div className="panel-title">输出位置</div>
      <Radio.Group value={value.mode} onChange={(e) => onChange({ ...value, mode: e.target.value })}>
        <Radio value="suffix">原图同目录</Radio>
        <Radio value="dir">指定目录</Radio>
      </Radio.Group>
      {value.mode === 'suffix' ? (
        <Input
          aria-label="文件名后缀"
          size="small"
          prefix={<span className="muted">文件名后缀</span>}
          value={value.suffix}
          onChange={(e) => onChange({ ...value, suffix: e.target.value })}
          style={{ marginTop: 8 }}
        />
      ) : (
        <Input
          aria-label="输出目录"
          size="small"
          readOnly
          value={value.dir ?? ''}
          placeholder="点击右侧选择目录"
          style={{ marginTop: 8 }}
          suffix={<Button type="text" size="small" icon={<FolderOpenOutlined />} onClick={pickDir} aria-label="选择目录" />}
        />
      )}
      <div className="panel-hint block">
        {value.mode === 'suffix' ? '转换格式时写为同名新扩展名；不会覆盖原图，重名自动加序号' : '重名文件自动加序号'}
      </div>
    </div>
  );
}
