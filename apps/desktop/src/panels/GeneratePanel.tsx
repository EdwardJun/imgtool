import { Input, Button } from 'antd';

/** 图片生成：接口接入前的占位面板，Rust 侧已有 imggen::ImageProvider 扩展点 */
export function GeneratePanel() {
  return (
    <div className="panel-section">
      <div className="panel-title">
        图片生成 <span className="badge-soon">即将上线</span>
      </div>
      <Input.TextArea aria-label="生成描述" rows={5} placeholder="描述你想生成的图片…" disabled />
      <Button type="primary" block disabled style={{ marginTop: 12 }}>
        生成
      </Button>
      <div className="panel-hint block">
        生成服务接口接入后可在这里使用。生成结果会按当前压缩设置自动处理，并支持写入项目目录。
      </div>
    </div>
  );
}
