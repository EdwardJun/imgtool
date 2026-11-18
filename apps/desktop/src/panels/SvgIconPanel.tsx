import { useEffect, useState } from 'react';
import { App, Button, ColorPicker, Input, InputNumber, Radio, Select, Tooltip } from 'antd';
import { QuestionCircleOutlined, SettingOutlined, ThunderboltOutlined } from '@ant-design/icons';
import { DEFAULT_PROMPT_MODEL, type ApiForm } from '../generate/useGenerator';
import { loadSvgForm, MAX_ICONS, parseList, saveSvgForm, SIZE_PRESETS, type SvgForm, type svgApi } from '../generate/svgIcons';

interface Props {
  api: ApiForm;
  onOpenSettings: () => void;
  running: boolean;
  onGenerate: (params: Parameters<typeof svgApi.generate>[0]) => void;
}

/** 线性 / 面性 SVG 图标：用对话模型一次生成整套，本地渲染多尺寸位图 */
export function SvgIconPanel({ api, onOpenSettings, running, onGenerate }: Props) {
  const { message } = App.useApp();
  const [form, setForm] = useState<SvgForm>(loadSvgForm);
  useEffect(() => saveSvgForm(form), [form]);

  const icons = parseList(form.list);
  const setStyle = (p: Partial<SvgForm['style']>) => setForm((f) => ({ ...f, style: { ...f.style, ...p } }));
  const setRender = (p: Partial<SvgForm['render']>) => setForm((f) => ({ ...f, render: { ...f.render, ...p } }));
  const model = api.prompt_model || DEFAULT_PROMPT_MODEL;

  const submit = () => {
    if (!api.base_url.trim() || !api.api_key.trim()) {
      message.warning('请先在「设置」里填写接口地址和 API Key');
      onOpenSettings();
      return;
    }
    if (!icons.length) return message.warning('请至少填写一个图标名称');
    if (icons.length > MAX_ICONS) return message.warning(`一次最多 ${MAX_ICONS} 个图标，请分批生成`);
    if (!form.render.sizes.length) return message.warning('请至少选择一个导出尺寸');
    onGenerate({
      base_url: api.base_url.trim(),
      api_key: api.api_key.trim(),
      model: api.prompt_model,
      icons,
      style: { ...form.style, notes: form.style.notes?.trim() || null },
      ...form.render,
    });
  };

  return (
    <div className="panel-section gen-panel">
      <div className="gen-api-status">
        {api.api_key ? <span className="muted">对话模型：{model}</span> : <span className="panel-hint warn">还没有配置 API Key</span>}
        <Button size="small" type="link" icon={<SettingOutlined />} onClick={onOpenSettings}>
          {api.api_key ? '设置' : '去设置'}
        </Button>
      </div>

      <div className="panel-title switch-row">
        <span>
          图标清单{' '}
          <Tooltip title="每行一个图标，也可以用逗号分隔。需要说明画法时写成「名称：说明」，例如「发现：指南针」。名称是英文时直接用作文件名，中文名会用模型给的英文短名">
            <QuestionCircleOutlined className="muted" />
          </Tooltip>
        </span>
        <span className={`panel-hint ${icons.length > MAX_ICONS ? 'warn' : ''}`}>
          {icons.length}/{MAX_ICONS}
        </span>
      </div>
      <Input.TextArea
        aria-label="图标清单"
        rows={5}
        value={form.list}
        onChange={(e) => setForm((f) => ({ ...f, list: e.target.value }))}
        placeholder={'首页\n发现：指南针\n购物车\n我的'}
      />

      <div className="panel-title">风格</div>
      <Radio.Group
        aria-label="图标类型"
        optionType="button"
        buttonStyle="solid"
        size="small"
        value={form.style.variant}
        onChange={(e) => setStyle({ variant: e.target.value })}
        options={[
          { value: 'outline', label: '线性' },
          { value: 'filled', label: '面性' },
        ]}
      />
      <div className="param-grid">
        {form.style.variant === 'outline' && (
          <label className="param">
            <span>线宽</span>
            <InputNumber aria-label="线宽" min={0.5} max={4} step={0.25} value={form.style.stroke_width} onChange={(v) => setStyle({ stroke_width: v ?? 2 })} style={{ width: '100%' }} />
          </label>
        )}
        <label className="param">
          <span>端点</span>
          <Select
            aria-label="端点与转角"
            value={form.style.corner}
            onChange={(corner) => setStyle({ corner })}
            options={[
              { value: 'round', label: '圆角' },
              { value: 'square', label: '直角' },
            ]}
          />
        </label>
        <label className="param">
          <span>颜色</span>
          <ColorPicker aria-label="图标颜色" value={form.render.color} disabledAlpha showText onChangeComplete={(c) => setRender({ color: c.toHexString().toUpperCase() })} />
        </label>
      </div>
      <Input
        aria-label="额外风格要求"
        placeholder="额外要求（可选），如：圆润可爱、细节少"
        value={form.style.notes ?? ''}
        onChange={(e) => setStyle({ notes: e.target.value })}
      />

      <div className="panel-title">导出</div>
      <div className="param-grid">
        <label className="param wide">
          <span>位图尺寸</span>
          <Select
            aria-label="导出尺寸"
            mode="tags"
            value={form.render.sizes.map(String)}
            onChange={(v: string[]) => {
              const sizes = [...new Set(v.map((s) => parseInt(s, 10)).filter((n) => n >= 8 && n <= 4096))].sort((a, b) => a - b);
              setRender({ sizes: sizes.slice(0, 12) });
            }}
            options={SIZE_PRESETS.map((s) => ({ value: String(s), label: `${s}px` }))}
            tokenSeparators={[',', ' ']}
            placeholder="可输入自定义尺寸"
          />
        </label>
        <label className="param">
          <span>位图格式</span>
          <Select
            aria-label="位图格式"
            value={form.render.format}
            onChange={(format) => setRender({ format })}
            options={[
              { value: 'png', label: 'png' },
              { value: 'webp', label: 'webp' },
            ]}
          />
        </label>
      </div>
      <div className="panel-hint block">每个图标导出 .svg 和所选尺寸的 {form.render.format}；生成后可直接换颜色、改尺寸，不用重新生成。</div>

      <div className="gen-submit">
        <Button type="primary" size="large" block icon={<ThunderboltOutlined />} loading={running} onClick={submit}>
          {running ? '生成中…' : `生成 ${icons.length || ''} 个图标`}
        </Button>
      </div>
    </div>
  );
}
