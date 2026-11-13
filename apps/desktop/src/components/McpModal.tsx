import { useEffect, useState } from 'react';
import { Alert, App, Button, List, Modal, Tag, Typography } from 'antd';
import { api } from '../api';
import type { McpClient, McpStatus } from '../types';

interface Props {
  open: boolean;
  onClose: () => void;
}

/** Agent 接入：把 imgtool MCP Server 一键写入 Claude Code / Cursor / Kiro 的用户级配置 */
export function McpModal({ open, onClose }: Props) {
  const { message } = App.useApp();
  const [status, setStatus] = useState<McpStatus | null>(null);
  const [busy, setBusy] = useState<McpClient | null>(null);

  const refresh = () => api.mcpStatus().then(setStatus).catch((e) => message.error(String(e)));

  useEffect(() => {
    if (open) refresh();
  }, [open]);

  const install = async (client: McpClient, name: string) => {
    setBusy(client);
    try {
      const path = await api.mcpInstall(client);
      message.success(`已写入 ${path}，重启 ${name} 后生效`);
      await refresh();
    } catch (e) {
      message.error(String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <Modal open={open} onCancel={onClose} footer={null} title="Agent 接入（MCP）" width={620}>
      <Typography.Paragraph type="secondary">
        imgtool 会作为 MCP Server 运行。Agent 可以扫描当前项目的图片、按项目预设 .imgtool.toml 压缩和转换格式。读写范围限制在 Agent 打开的项目目录内，默认不覆盖原图。
      </Typography.Paragraph>
      {status?.error && <Alert type="warning" showIcon message={status.error} style={{ marginBottom: 12 }} />}
      <List
        bordered
        dataSource={status?.clients ?? []}
        renderItem={(c) => (
          <List.Item
            actions={[
              <Button
                key="install"
                size="small"
                type={c.up_to_date ? 'default' : 'primary'}
                loading={busy === c.id}
                disabled={!status?.command}
                onClick={() => install(c.id, c.name)}
              >
                {c.up_to_date ? '重新写入' : c.configured ? '更新配置' : '写入配置'}
              </Button>,
            ]}
          >
            <List.Item.Meta
              title={
                <span>
                  {c.name}{' '}
                  {c.up_to_date ? <Tag color="success">已接入</Tag> : c.configured ? <Tag color="warning">路径已变化</Tag> : <Tag>未接入</Tag>}
                </span>
              }
              description={<Typography.Text type="secondary" style={{ fontSize: 12 }}>{c.config_path}</Typography.Text>}
            />
          </List.Item>
        )}
      />
      {status?.snippet && (
        <>
          <Typography.Title level={5} style={{ marginTop: 16 }}>
            其他客户端手动配置
          </Typography.Title>
          <Typography.Paragraph copyable={{ text: status.snippet }} style={{ marginBottom: 0 }}>
            <pre className="snippet">{status.snippet}</pre>
          </Typography.Paragraph>
          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            首次写入会备份原配置文件（*.imgtool.bak）。
          </Typography.Text>
        </>
      )}
    </Modal>
  );
}
