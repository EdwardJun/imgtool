import { useEffect, useMemo, useState } from 'react';
import { App, AutoComplete, Button, Input, InputNumber, Select, Switch, Tooltip } from 'antd';
import {
  CloseOutlined,
  CloudUploadOutlined,
  LoadingOutlined,
  QuestionCircleOutlined,
  ReloadOutlined,
  SettingOutlined,
  StarOutlined,
  ThunderboltOutlined,
  UndoOutlined,
} from '@ant-design/icons';
import { open } from '@tauri-apps/plugin-dialog';
import { api as backend, fileSrc, IMAGE_EXTENSIONS } from '../api';
import { FORMAT_LABEL, OUTPUT_FORMATS, type BackgroundRemoval, type GenerateParams } from '../types';
import { isHfsyapi, matchRatioTier, MODELS, PROMPT_MAX, sizeFor, specFor, type Tier } from '../generate/models';
import { DEFAULT_PROMPT_MODEL, loadForm, saveForm, type ApiForm, type GenForm } from '../generate/useGenerator';

interface Props {
  /** 已保存的接口设置（在「设置」窗口里修改） */
  api: ApiForm;
  onOpenSettings: () => void;
  running: boolean;
  onGenerate: (params: GenerateParams, count: number) => void;
  /** 「背景处理」页当前的去背景参数 */
  removal: BackgroundRemoval;
  onEditRemoval: () => void;
}

const isUrl = (s: string) => /^https?:\/\//i.test(s);
const UPLOAD_CONSENT_KEY = 'imgtool.uploadConsent';

type UploadState = { state: 'uploading' } | { state: 'done'; url: string; expiresAt: number } | { state: 'error'; error: string };

export function GeneratePanel({ running, onGenerate, removal, onEditRemoval, api, onOpenSettings }: Props) {
  const { message, modal } = App.useApp();
  const [form, setForm] = useState<GenForm>(loadForm);
  const [prompt, setPrompt] = useState('');
  const [enhancing, setEnhancing] = useState(false);
  /** 优化前的原始描述，用于撤销 */
  const [original, setOriginal] = useState<string | null>(null);
  const [refs, setRefs] = useState<string[]>([]);
  const [refInput, setRefInput] = useState('');
  const [uploads, setUploads] = useState<Record<string, UploadState>>({});

  useEffect(() => saveForm(form), [form]);

  const spec = specFor(form.model);
  const urlOnly = isHfsyapi(api.base_url);
  // 切换模型后，让比例/档位落在该模型支持的范围内
  useEffect(() => {
    setForm((f) => ({
      ...f,
      ratio: spec.ratios.includes(f.ratio) ? f.ratio : spec.ratios[0],
      tier: spec.tiers.includes(f.tier) ? f.tier : spec.tiers[0],
      transparent: spec.transparent ? f.transparent : false,
    }));
  }, [spec.id]);

  const size = sizeFor(form.ratio, form.tier);
  const set = (p: Partial<GenForm>) => setForm((f) => ({ ...f, ...p }));
  const fs = form.finalSize;
  const best = fs.width && fs.height ? matchRatioTier(spec, fs.width, fs.height) : null;
  const matched = !best || (best.ratio === form.ratio && best.tier === form.tier);
  const upscale = fs.enabled && !!fs.width && !!fs.height && (fs.width > size.width * 1.05 || fs.height > size.height * 1.05);

  // 修改最终尺寸时自动选最接近的比例和够用的档位（之后仍可手动改）
  const setFinal = (p: Partial<GenForm['finalSize']>) =>
    setForm((f) => {
      const next = { ...f.finalSize, ...p };
      const sizeChanged = 'width' in p || 'height' in p || (p.enabled && !f.finalSize.enabled);
      if (next.enabled && sizeChanged && next.width && next.height) {
        return { ...f, finalSize: next, ...matchRatioTier(specFor(f.model), next.width, next.height) };
      }
      return { ...f, finalSize: next };
    });
  const applyMatch = () => best && set(best);

  const modelOptions = useMemo(() => {
    const groups = [...new Set(MODELS.map((x) => x.group))];
    return groups.map((g) => ({ label: g, options: MODELS.filter((x) => x.group === g).map((x) => ({ value: x.id, label: x.id })) }));
  }, []);

  /** 上传本地参考图到 hfsyapi 图床，状态显示在缩略图上 */
  const uploadRef = async (path: string) => {
    const base = api.base_url.trim();
    const key = api.api_key.trim();
    if (!key) {
      setUploads((u) => ({ ...u, [path]: { state: 'error', error: '请先在「设置」里填写 API Key' } }));
      return;
    }
    setUploads((u) => ({ ...u, [path]: { state: 'uploading' } }));
    try {
      const r = await backend.uploadReference(base, key, path);
      setUploads((u) => ({ ...u, [path]: { state: 'done', url: r.url, expiresAt: r.expires_at } }));
    } catch (e) {
      setUploads((u) => ({ ...u, [path]: { state: 'error', error: String(e) } }));
    }
  };

  const addRefs = (items: string[]) => {
    const fresh = items.filter((i) => i && !refs.includes(i));
    const merged = [...refs, ...fresh];
    if (merged.length > spec.maxRefs) message.warning(`当前模型参考图最多 ${spec.maxRefs} 张，已忽略多余的`);
    const kept = merged.slice(0, spec.maxRefs);
    setRefs(kept);
    if (urlOnly) fresh.filter((p) => !isUrl(p) && kept.includes(p)).forEach(uploadRef);
  };

  const removeRef = (r: string) => {
    setRefs((prev) => prev.filter((x) => x !== r));
    setUploads(({ [r]: _removed, ...rest }) => rest);
  };

  /** 首次上传本地图片前说明：图片会传到第三方存储，链接 24 小时内可被访问 */
  const confirmUpload = () =>
    new Promise<boolean>((resolve) => {
      if (!urlOnly || localStorage.getItem(UPLOAD_CONSENT_KEY)) return resolve(true);
      modal.confirm({
        title: '本地图片将上传到 hfsyapi 图床',
        content: 'hfsyapi 只接受图片地址作为参考图，所以本地图片会先上传到它的免费图床（较大的图会先压缩到长边 2048px）。上传后得到的链接 24 小时内有效，拿到链接的人都能访问这张图。',
        okText: '继续上传',
        cancelText: '取消',
        onOk: () => {
          localStorage.setItem(UPLOAD_CONSENT_KEY, '1');
          resolve(true);
        },
        onCancel: () => resolve(false),
      });
    });

  const addUrl = () => {
    const parts = refInput.split(/[\s,]+/).map((s) => s.trim()).filter(Boolean);
    const bad = parts.filter((p) => !isUrl(p));
    if (bad.length) return message.warning('请输入以 http(s):// 开头的图片地址，本地图片请用"选择本地图片"');
    addRefs(parts);
    setRefInput('');
  };

  const pickLocal = async () => {
    if (!(await confirmUpload())) return;
    const picked = await open({ multiple: true, title: '选择参考图片', filters: [{ name: '图片', extensions: IMAGE_EXTENSIONS }] });
    if (Array.isArray(picked)) addRefs(picked);
    else if (typeof picked === 'string') addRefs([picked]);
  };

  /** 用对话大模型把简短描述改写成专业提示词，带上本次生成参数作为上下文 */
  const enhance = async () => {
    if (!api.api_key.trim()) {
      message.warning('请先在「设置」里填写 API Key');
      onOpenSettings();
      return;
    }
    const source = prompt.trim();
    if (!source) return;
    setEnhancing(true);
    try {
      const r = await backend.enhancePrompt({
        base_url: api.base_url,
        api_key: api.api_key,
        model: api.prompt_model,
        prompt: source,
        context: {
          image_model: form.model.trim(),
          ratio: form.ratio,
          size: `${size.width}x${size.height}`,
          remove_background: form.removeBg || (spec.transparent && form.transparent),
          reference_count: refs.length,
          final_size: fs.enabled && fs.width && fs.height ? `${fs.width}x${fs.height}` : null,
        },
      });
      setOriginal(source);
      setPrompt(r.prompt);
      message.success(`已优化（${r.model}）`);
    } catch (e) {
      message.error(String(e));
    } finally {
      setEnhancing(false);
    }
  };

  const undoEnhance = () => {
    if (original === null) return;
    setPrompt(original);
    setOriginal(null);
  };

  const submit = () => {
    const base = api.base_url.trim();
    const key = api.api_key.trim();
    if (!base || !key) {
      message.warning('请先在「设置」里填写接口地址和 API Key');
      onOpenSettings();
      return;
    }
    if (!form.model.trim()) return message.warning('请选择模型');
    if (!prompt.trim()) return message.warning('请输入提示词');
    if (refs.length > spec.maxRefs) return message.warning(`当前模型参考图最多 ${spec.maxRefs} 张`);
    if (urlOnly) {
      const local = refs.filter((r) => !isUrl(r));
      if (local.some((r) => uploads[r]?.state === 'uploading')) return message.warning('参考图还在上传，请稍等');
      const failed = local.filter((r) => uploads[r]?.state === 'error');
      if (failed.length) return message.warning(`有 ${failed.length} 张参考图上传失败，请点击缩略图重试或移除`);
      // 未上传过的（例如切换接口地址后）由后端在生成前自动上传
    }
    onGenerate(
      {
        base_url: base,
        api_key: key,
        model: form.model.trim(),
        prompt: prompt.trim(),
        width: size.width,
        height: size.height,
        reference_images: refs,
        transparent: spec.transparent && form.transparent,
        format: form.format,
        remove_background: form.removeBg ? removal : null,
        final_size: fs.enabled && fs.width && fs.height ? { width: fs.width, height: fs.height, fit: fs.fit } : null,
        analyze_design: form.analyzeDesign,
      },
      form.count,
    );
  };

  let host = '';
  try {
    host = new URL(api.base_url).host;
  } catch {
    host = api.base_url;
  }

  return (
    <div className="panel-section gen-panel">
      <div className="gen-api-status">
        {api.api_key ? (
          <span className="muted">接口：{host || '未填写地址'}</span>
        ) : (
          <span className="panel-hint warn">还没有配置 API Key</span>
        )}
        <Button size="small" type="link" icon={<SettingOutlined />} onClick={onOpenSettings}>
          {api.api_key ? '设置' : '去设置'}
        </Button>
      </div>

      <div className="panel-title switch-row">
        <span>提示词 Prompt</span>
        {original !== null && !enhancing && (
          <Button size="small" type="link" icon={<UndoOutlined />} onClick={undoEnhance}>
            撤销优化
          </Button>
        )}
      </div>
      <div className="prompt-box">
        <Input.TextArea
          aria-label="提示词"
          rows={6}
          maxLength={PROMPT_MAX}
          showCount
          value={prompt}
          disabled={enhancing}
          onChange={(e) => {
            setPrompt(e.target.value);
            // 用户手动改过优化结果后不再提供撤销，避免覆盖修改
            if (original !== null) setOriginal(null);
          }}
          placeholder="描述你想要生成的图片，例如：一只在月球上弹吉他的猫，赛博朋克风格…"
        />
        <Tooltip title={`用 ${api.prompt_model || DEFAULT_PROMPT_MODEL} 结合当前参数把描述改写成专业提示词`}>
          <Button className="prompt-enhance" size="small" icon={<StarOutlined />} loading={enhancing} disabled={!prompt.trim()} onClick={enhance}>
            {enhancing ? '优化中…' : '优化提示词'}
          </Button>
        </Tooltip>
      </div>

      <div className="panel-title">
        参考图片（可选） <span className="panel-hint warn">当前模型最多 {spec.maxRefs} 张</span>
      </div>
      <Input
        aria-label="参考图片 URL"
        placeholder="粘贴图片 URL，回车添加"
        value={refInput}
        onChange={(e) => setRefInput(e.target.value)}
        onPressEnter={addUrl}
      />
      <div className="switch-row">
        <Tooltip title={urlOnly ? '本地图片会先上传到 hfsyapi 图床（免费，链接 24 小时有效）' : '本地图片会以 data URL 发送'}>
          <Button size="small" onClick={pickLocal}>
            选择本地图片
          </Button>
        </Tooltip>
        <Button
          size="small"
          onClick={() => {
            setRefs([]);
            setUploads({});
          }}
          disabled={!refs.length}
        >
          清除
        </Button>
      </div>
      {refs.length > 0 && (
        <div className="ref-list">
          {refs.map((r) => {
            const up = urlOnly && !isUrl(r) ? uploads[r] : undefined;
            const tip =
              up?.state === 'error'
                ? `${up.error}（点击重试）`
                : up?.state === 'done'
                  ? `已上传，链接有效至 ${new Date(up.expiresAt * 1000).toLocaleString()}`
                  : up?.state === 'uploading'
                    ? '上传中…'
                    : r;
            return (
              <Tooltip key={r} title={tip}>
                <div className={`ref-item ${up?.state ?? ''}`}>
                  <img src={isUrl(r) ? r : fileSrc(r)} alt="参考图" />
                  {up?.state === 'uploading' && (
                    <span className="ref-badge">
                      <LoadingOutlined />
                    </span>
                  )}
                  {up?.state === 'done' && (
                    <span className="ref-badge ok" aria-label="已上传">
                      <CloudUploadOutlined />
                    </span>
                  )}
                  {up?.state === 'error' && (
                    <button className="ref-badge err" aria-label="上传失败，点击重试" onClick={() => uploadRef(r)}>
                      <ReloadOutlined />
                    </button>
                  )}
                  <button className="ref-remove" aria-label="移除参考图" onClick={() => removeRef(r)}>
                    <CloseOutlined />
                  </button>
                </div>
              </Tooltip>
            );
          })}
        </div>
      )}

      <div className="panel-title">参数设置</div>
      <div className="param-grid">
        <label className="param">
          <span>模型</span>
          <AutoComplete
            aria-label="模型"
            value={form.model}
            options={modelOptions}
            onChange={(v) => set({ model: v })}
            // 已选中完整模型名时显示全部选项，方便切换；否则按输入过滤
            filterOption={(input, opt) =>
              !input || MODELS.some((x) => x.id === input) || String((opt as { value?: string } | undefined)?.value ?? '').includes(input)
            }
            placeholder="选择或输入模型名"
          />
        </label>
        <label className="param">
          <span>比例</span>
          <Select aria-label="比例" value={form.ratio} onChange={(v) => set({ ratio: v })} options={spec.ratios.map((r) => ({ value: r }))} />
        </label>
        <label className="param">
          <span>质量</span>
          <Select aria-label="质量" value={form.tier} onChange={(v) => set({ tier: v as Tier })} options={spec.tiers.map((t) => ({ value: t, label: t.toLowerCase() }))} />
        </label>
        <label className="param">
          <span>生成格式</span>
          <Select aria-label="生成格式" value={form.format} onChange={(v) => set({ format: v })} options={OUTPUT_FORMATS.map((f) => ({ value: f, label: FORMAT_LABEL[f].toLowerCase() }))} />
        </label>
        <label className="param">
          <span>生成数量</span>
          <InputNumber aria-label="生成数量" min={1} max={4} value={form.count} onChange={(v) => set({ count: v ?? 1 })} style={{ width: '100%' }} />
        </label>
        {spec.transparent && (
          <label className="param">
            <span>透明背景</span>
            <Switch aria-label="透明背景" checked={form.transparent} onChange={(v) => set({ transparent: v })} />
          </label>
        )}
      </div>
      <div className="switch-row gen-removebg">
        <span>
          生成后去除背景
          <Button size="small" type="link" onClick={onEditRemoval}>
            调整参数
          </Button>
        </span>
        <Switch aria-label="生成后去除背景" checked={form.removeBg} onChange={(v) => set({ removeBg: v })} />
      </div>
      {form.removeBg && (
        <div className="panel-hint block">
          使用「背景处理」页的去背景参数（容差 {removal.tolerance}、柔和 {removal.softness}
          {removal.trim ? '、裁掉空白' : ''}）。提示词里写明"纯白背景"效果最好。
        </div>
      )}
      <div className="switch-row">
        <span>
          生成后解析设计稿{' '}
          <Tooltip title="生成 UI 设计稿时开启：自动解析成节点坐标、样式与布局，并裁出图标素材，可在「设计解析」里查看和标注">
            <QuestionCircleOutlined className="muted" />
          </Tooltip>
        </span>
        <Switch aria-label="生成后解析设计稿" checked={form.analyzeDesign} onChange={(v) => set({ analyzeDesign: v })} />
      </div>
      <div className="switch-row">
        <span>
          调整到最终尺寸{' '}
          <Tooltip title="接口只能按比例 × 1K/2K/4K 的固定几档出图（实际尺寸也可能有偏差），开启后会在本地把结果调整到这里设置的尺寸">
            <QuestionCircleOutlined className="muted" />
          </Tooltip>
        </span>
        <Switch aria-label="调整到最终尺寸" checked={fs.enabled} onChange={(v) => setFinal({ enabled: v })} />
      </div>
      {fs.enabled && (
        <>
          <div className="size-row">
            <InputNumber aria-label="最终宽度" min={8} max={8192} value={fs.width} suffix="宽" onChange={(v) => setFinal({ width: v })} style={{ flex: 1 }} />
            <span className="lock-btn static">×</span>
            <InputNumber aria-label="最终高度" min={8} max={8192} value={fs.height} suffix="高" onChange={(v) => setFinal({ height: v })} style={{ flex: 1 }} />
          </div>
          <Select
            aria-label="调整方式"
            value={fs.fit}
            onChange={(fit) => setFinal({ fit })}
            options={[
              { value: 'cover', label: '裁剪填满（精确尺寸，居中裁掉多余部分）' },
              { value: 'contain', label: '完整缩放（等比，不超过该尺寸）' },
              { value: 'fill', label: '拉伸（精确尺寸，可能变形）' },
            ]}
          />
          <div className="switch-row">
            <span className="panel-hint">
              按 {form.ratio} · {form.tier.toLowerCase()} 生成，再调整到 {fs.width ?? '?'}×{fs.height ?? '?'}
            </span>
            {!matched && (
              <Button size="small" type="link" onClick={applyMatch}>
                按最终尺寸选比例
              </Button>
            )}
          </div>
          {upscale && <div className="panel-hint warn block">生成尺寸 {size.width}×{size.height} 小于最终尺寸，放大后可能变模糊，可选更高质量档位</div>}
        </>
      )}
      <div className="panel-hint block">
        {fs.enabled ? '生成尺寸' : '输出尺寸'} {size.width}×{size.height}
        {spec.group === '自定义' && '（自定义模型，尺寸是否支持以接口返回为准）'}
        {(form.transparent || form.removeBg) && form.format === 'jpeg' && '；JPG 不支持透明，将改存 PNG'}
      </div>

      {/* 固定在面板底部，不用滚动就能点到 */}
      <div className="gen-submit">
        <Button type="primary" size="large" block icon={<ThunderboltOutlined />} loading={running} onClick={submit}>
          {running ? '生成中…' : '生成图片'}
        </Button>
      </div>
    </div>
  );
}
