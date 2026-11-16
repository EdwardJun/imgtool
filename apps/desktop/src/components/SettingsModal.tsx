import { useEffect, useState } from 'react';
import { App, AutoComplete, Button, Checkbox, Form, Input, Modal, Typography } from 'antd';
import { DEFAULT_BASE_URL } from '../generate/models';
import { DEFAULT_PROMPT_MODEL, type ApiForm } from '../generate/useGenerator';

/** hfsyapi 上的对话模型（deepseek-v4.1-flash、gpt-6.1-sol 已实测可用，其余按模型列表提供） */
const PROMPT_MODELS = ['deepseek-v4.1-flash', 'gpt-6.1-sol', 'gpt-5.6-sol', 'deepseek-v4-pro', 'kimi-k3.1-pro', 'claude-sonnet-5'];

interface Props {
  open: boolean;
  value: ApiForm;
  onClose: () => void;
  onSave: (v: ApiForm) => Promise<void>;
}

/** 全局设置：目前只有图片生成接口，后续的 AI 服务配置也放这里 */
export function SettingsModal({ open, value, onClose, onSave }: Props) {
  const { message } = App.useApp();
  const [draft, setDraft] = useState(value);
  const [saving, setSaving] = useState(false);

  // 每次打开都从已保存的值开始编辑
  useEffect(() => {
    if (open) setDraft(value);
  }, [open, value]);

  const save = async () => {
    const base = draft.base_url.trim();
    if (base && !/^https?:\/\//i.test(base)) return message.warning('接口地址需以 http:// 或 https:// 开头');
    setSaving(true);
    try {
      await onSave({ ...draft, base_url: base, api_key: draft.api_key.trim(), prompt_model: draft.prompt_model.trim() || DEFAULT_PROMPT_MODEL });
      message.success(draft.remember ? '设置已保存到本机' : '设置只在本次运行中有效，已清除本机保存的配置');
      onClose();
    } catch (e) {
      message.error(String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      open={open}
      title="设置"
      onCancel={onClose}
      width={520}
      footer={[
        <Button key="cancel" onClick={onClose}>
          取消
        </Button>,
        <Button key="save" type="primary" loading={saving} onClick={save}>
          保存
        </Button>,
      ]}
    >
      <div className="settings-section">
        <Typography.Title level={5} style={{ margin: 0 }}>
          图片生成接口
        </Typography.Title>
        <Form layout="vertical" requiredMark={false}>
          <Form.Item label="接口地址" style={{ marginBottom: 12 }}>
            <Input
              aria-label="接口地址"
              placeholder={DEFAULT_BASE_URL}
              value={draft.base_url}
              onChange={(e) => setDraft({ ...draft, base_url: e.target.value })}
              onPressEnter={save}
            />
          </Form.Item>
          <Form.Item label="API Key" style={{ marginBottom: 12 }}>
            <Input.Password
              aria-label="API Key"
              placeholder="sk-..."
              autoComplete="off"
              value={draft.api_key}
              onChange={(e) => setDraft({ ...draft, api_key: e.target.value })}
              onPressEnter={save}
            />
          </Form.Item>
          <Form.Item label="提示词优化模型" extra="用于「优化提示词」，走同一地址的 /v1/chat/completions，需要是对话模型" style={{ marginBottom: 12 }}>
            <AutoComplete
              aria-label="提示词优化模型"
              placeholder={DEFAULT_PROMPT_MODEL}
              value={draft.prompt_model}
              options={PROMPT_MODELS.map((m) => ({ value: m }))}
              onChange={(v) => setDraft({ ...draft, prompt_model: v })}
            />
          </Form.Item>
          <Checkbox checked={draft.remember} onChange={(e) => setDraft({ ...draft, remember: e.target.checked })}>
            记住到本机
          </Checkbox>
        </Form>
        <Typography.Text type="secondary" style={{ fontSize: 12 }}>
          兼容 hfsyapi 的任务接口（/v1/image/create + /v1/image/query）。Key 只保存在本机应用配置目录，不会写进项目文件。取消"记住到本机"后，Key 只在本次运行中有效。
        </Typography.Text>
      </div>
    </Modal>
  );
}
