import { useMemo } from 'react';
import { AutoComplete, Button, Checkbox, Empty, Input, InputNumber, Segmented, Select, Switch, Tag, Tooltip, Tree } from 'antd';
import { ApartmentOutlined, FolderOpenOutlined, QuestionCircleOutlined, ReloadOutlined, SaveOutlined, ThunderboltOutlined } from '@ant-design/icons';
import { fileSrc } from '../api';
import type { DesignSettings, Fill, SpecNode, NodeType } from '../design/types';
import { NODE_TYPES, ROLE_OPTIONS } from '../design/types';
import type { DesignState } from '../design/useDesign';
import { flatten, nodeLabel } from '../design/tree';

/** 与 imgspec::icons::MIN_REDRAW_PX 一致 */
const MIN_REDRAW_PX = 24;
const nativePx = (n: SpecNode) => Math.max(n.bounds.width, n.bounds.height);

interface Props {
  value: DesignSettings;
  onChange: (v: DesignSettings) => void;
  hasImage: boolean;
  state?: DesignState;
  selected: string | null;
  onSelect: (id: string | null) => void;
  onAnalyze: () => void;
  onEditNode: (id: string, patch: Partial<SpecNode>) => void;
  onNormalize: () => void;
  onSave: () => void;
  /** ids 为空时按后端默认规则选择（跳过过小的图标） */
  onGenerateIcons: (ids: string[] | null) => void;
  /** 已填写 API Key */
  apiReady: boolean;
  onOpenSettings: () => void;
  /** 在系统文件管理器里打开解析文件夹 */
  onOpenFolder: (dir: string) => void;
}

const fillText = (f?: Fill) => (!f ? '—' : f.type === 'solid' ? f.color : `线性 ${f.angle}° ${f.stops[0]?.color} → ${f.stops[f.stops.length - 1]?.color}`);

function Swatch({ color }: { color?: string }) {
  return color ? <span className="swatch" style={{ background: color }} /> : null;
}

function toTree(n: SpecNode): { key: string; title: string; children?: ReturnType<typeof toTree>[] } {
  return { key: n.id, title: nodeLabel(n), children: n.children?.length ? n.children.map(toTree) : undefined };
}

/** 节点的样式与布局（只读） */
function StyleInfo({ node }: { node: SpecNode }) {
  const s = node.style ?? {};
  const t = node.text_style;
  const l = node.layout;
  const b = node.bounds;
  const rows: [string, React.ReactNode][] = [
    ['位置', `${b.x}, ${b.y}（相对父节点 ${node.relative.x}, ${node.relative.y}）`],
    ['尺寸', `${b.width} × ${b.height}`],
  ];
  if (s.fill) rows.push(['填充', <><Swatch color={s.fill.type === 'solid' ? s.fill.color : s.fill.stops[0]?.color} /> {fillText(s.fill)}</>]);
  if (s.stroke) rows.push(['描边', <><Swatch color={s.stroke.color} /> {s.stroke.width}px {s.stroke.color}</>]);
  if (s.corner_radius !== undefined) rows.push(['圆角', Array.isArray(s.corner_radius) ? s.corner_radius.join(' / ') : `${s.corner_radius}px`]);
  if (s.shadow) rows.push(['阴影', `${s.shadow.x}px ${s.shadow.y}px ${s.shadow.blur}px ${s.shadow.color}（置信度 ${Math.round(s.shadow.confidence * 100)}%）`]);
  if (t)
    rows.push([
      '文字',
      <>
        <Swatch color={t.color} /> {t.color} · 约 {t.font_size}px · {t.weight === 'bold' ? '粗' : t.weight === 'medium' ? '中' : '常规'} · {t.lines} 行
        {t.line_height ? ` · 行高 ${t.line_height}` : ''}
      </>,
    ]);
  if (l)
    rows.push([
      '布局',
      `${l.mode === 'row' ? '横排' : l.mode === 'column' ? '竖排' : '自由'}${l.gap !== undefined ? ` · 间距 ${l.gap}${l.gap_uniform ? '' : '（不均匀）'}` : ''} · 内边距 ${l.padding.join('/')} · ${l.align}/${l.cross_align}`,
    ]);
  return (
    <dl className="ds-props">
      {rows.map(([k, v]) => (
        <div key={k}>
          <dt>{k}</dt>
          <dd>{v}</dd>
        </div>
      ))}
    </dl>
  );
}

/** 设计解析：参数、层级树、节点属性（可标注语义）、图标素材与 AI 重绘 */
export function DesignPanel(p: Props) {
  const { value, state } = p;
  const set = (v: Partial<DesignSettings>) => p.onChange({ ...value, ...v });
  const spec = state?.spec;
  const node = useMemo(() => (spec && p.selected ? flatten(spec.root).find((f) => f.node.id === p.selected)?.node ?? null : null), [spec, p.selected]);
  const icons = useMemo(() => (spec ? flatten(spec.root).filter((f) => f.node.type === 'icon').map((f) => f.node) : []), [spec]);
  const treeData = useMemo(() => (spec ? [toTree(spec.root)] : []), [spec]);
  const smallCount = icons.filter((n) => nativePx(n) < MIN_REDRAW_PX).length;
  // 要重绘的图标：默认跳过过小的（与后端默认规则一致），勾选后全部重绘
  const redrawIds = icons.filter((n) => value.iconForceSmall || nativePx(n) >= MIN_REDRAW_PX).map((n) => n.id);
  const unnamed = icons.filter((n) => redrawIds.includes(n.id) && !n.name && !n.role && !n.description).map((n) => n.id);
  const busy = state?.busy;
  const root = state?.result?.root ?? '';
  // spec 里的素材路径相对项目根目录；绝对路径（含 Windows 盘符）和 blob: 地址直接用
  const assetSrc = (path: string) => {
    const src = fileSrc(/^(\/|blob:|[A-Za-z]:[\\/])/.test(path) ? path : `${root}/${path}`);
    return src.startsWith('blob:') ? src : `${src}?v=${state?.rev ?? 0}`;
  };

  return (
    <div className="panel-section">
      <div className="panel-title">类型</div>
      <Segmented
        block
        value={value.mode}
        onChange={(v) => set({ mode: v as DesignSettings['mode'] })}
        options={[
          { label: '自动', value: 'auto' },
          { label: '整页设计稿', value: 'page' },
          { label: '单个样式', value: 'component' },
        ]}
      />
      <div className="panel-title">
        容差{' '}
        <Tooltip title="与底色差多少算作元素。细节（浅色卡片、细描边）被漏掉时调小，背景噪点被识别成元素时调大。">
          <QuestionCircleOutlined className="muted" />
        </Tooltip>
      </div>
      <div className="slider-row">
        <Checkbox checked={value.tolerance === null} onChange={(e) => set({ tolerance: e.target.checked ? null : 3 })}>
          自动
        </Checkbox>
        {value.tolerance !== null && (
          <InputNumber aria-label="容差" min={0} max={30} value={value.tolerance} onChange={(v) => set({ tolerance: v ?? 0 })} style={{ flex: 1 }} />
        )}
      </div>
      <div className="switch-row">
        <span>自动裁出图标素材</span>
        <Switch aria-label="自动裁出图标素材" checked={value.exportIcons} onChange={(v) => set({ exportIcons: v })} />
      </div>
      <Button block type={spec ? 'default' : 'primary'} icon={<ReloadOutlined />} loading={busy === 'analyze'} disabled={!p.hasImage} onClick={p.onAnalyze}>
        {spec ? '重新解析（保留标注）' : '解析设计稿'}
      </Button>
      {state?.error && <div className="stats-error">{state.error}</div>}

      {spec && (
        <>
          <div className="panel-title switch-row">
            <span>
              层级 <span className="panel-hint">{flatten(spec.root).length} 个节点</span>
            </span>
            <span className="ds-view">
              <Checkbox checked={value.showGroups} onChange={(e) => set({ showGroups: e.target.checked })}>
                显示分组
              </Checkbox>
              <Select
                size="small"
                aria-label="显示层数"
                value={value.depth}
                onChange={(v) => set({ depth: v })}
                options={[0, 1, 2, 3, 4].map((d) => ({ value: d, label: d === 0 ? '全部层' : `前 ${d} 层` }))}
                style={{ width: 88 }}
              />
            </span>
          </div>
          <div className="ds-tree">
            <Tree
              treeData={treeData}
              selectedKeys={p.selected ? [p.selected] : []}
              defaultExpandAll
              blockNode
              onSelect={(keys) => p.onSelect((keys[0] as string) ?? null)}
            />
          </div>

          {node ? (
            <>
              <div className="panel-title">
                节点 {node.id} <Tag>{node.source === 'auto' ? '自动识别' : node.source === 'agent' ? 'Agent 标注' : '手动修改'}</Tag>
              </div>
              <div className="ds-form">
                <label>
                  类型
                  <Select size="small" value={node.type} options={NODE_TYPES} onChange={(v: NodeType) => p.onEditNode(node.id, { type: v })} />
                </label>
                <label>
                  角色
                  <AutoComplete
                    size="small"
                    value={node.role ?? ''}
                    options={ROLE_OPTIONS.map((r) => ({ value: r }))}
                    placeholder="如 button、card、title"
                    onChange={(v) => p.onEditNode(node.id, { role: v || undefined })}
                    filterOption={(input, o) => String(o?.value ?? '').includes(input)}
                  />
                </label>
                <label>
                  名称
                  <Input size="small" value={node.name ?? ''} placeholder={node.type === 'icon' ? '素材文件名，如 home' : '如 登录按钮'} onChange={(e) => p.onEditNode(node.id, { name: e.target.value || undefined })} />
                </label>
                {(node.type === 'text' || node.text) && (
                  <label>
                    文字
                    <Input.TextArea size="small" autoSize={{ minRows: 1, maxRows: 4 }} value={node.text ?? ''} placeholder="画面上的文字原文" onChange={(e) => p.onEditNode(node.id, { text: e.target.value || undefined })} />
                  </label>
                )}
                <label>
                  说明
                  <Input.TextArea size="small" autoSize={{ minRows: 1, maxRows: 4 }} value={node.description ?? ''} placeholder="用途、交互、状态" onChange={(e) => p.onEditNode(node.id, { description: e.target.value || undefined })} />
                </label>
              </div>
              <StyleInfo node={node} />
              {node.asset && (
                <div className="ds-asset">
                  <img className="checkerboard" src={assetSrc(node.asset.path)} alt={node.name ?? node.id} />
                  <span className="muted">{node.asset.source === 'generated' ? 'AI 高清版' : '原图裁剪'} · {node.asset.path}</span>
                </div>
              )}
            </>
          ) : (
            <div className="panel-hint block">点击左侧图上的框或上方层级选中节点，可修改类型、补充角色 / 名称 / 文字。</div>
          )}

          <div className="ds-actions">
            <Tooltip title="按包含关系重新嵌套、重新推断布局（编号不变）。移动、新增框后使用">
              <Button icon={<ApartmentOutlined />} loading={busy === 'normalize'} onClick={p.onNormalize}>
                整理层级
              </Button>
            </Tooltip>
            <Button type={state?.dirty ? 'primary' : 'default'} icon={<SaveOutlined />} loading={busy === 'save'} disabled={!state?.dirty} onClick={p.onSave}>
              {state?.dirty ? '保存修改' : '已保存'}
            </Button>
          </div>

          <div className="panel-title">
            图标素材 <span className="panel-hint">{icons.length} 个</span>
          </div>
          {icons.length ? (
            <div className="ds-icons">
              {icons.map((n) => {
                const small = nativePx(n) < MIN_REDRAW_PX;
                return (
                  <button
                    key={n.id}
                    className={`ds-icon ${n.id === p.selected ? 'selected' : ''} ${small ? 'low-res' : ''}`}
                    onClick={() => p.onSelect(n.id)}
                    title={`${n.asset?.path ?? '保存后生成素材'}\n原图 ${n.bounds.width}×${n.bounds.height}px${small ? '：太小，放大后会糊，默认不参与 AI 重绘' : ''}`}
                  >
                    {n.asset ? <img className="checkerboard" src={assetSrc(n.asset.path)} alt={n.name ?? n.id} /> : <span className="ds-icon-empty">{n.id}</span>}
                    <span className="ds-icon-name">
                      {n.name || n.id}
                      {n.asset?.source === 'generated' && <ThunderboltOutlined className="ds-hd" aria-label="AI 高清版" />}
                    </span>
                    <span className="ds-icon-size">
                      {nativePx(n)}px{small && <Tag className="ds-small-tag">小</Tag>}
                    </span>
                  </button>
                );
              })}
            </div>
          ) : (
            <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="没有识别到图标；可把节点类型改为「图标」" />
          )}
          {icons.length > 0 && (
            <>
              <Input size="small" value={value.iconStyle} placeholder="重绘风格（可选），如 线性图标，2px 线宽，圆角端点" onChange={(e) => set({ iconStyle: e.target.value })} />
              <AutoComplete
                size="small"
                value={value.iconModel}
                options={['gpt-image-2', 'gpt-image-2.5-flare', 'nano-banana-2', 'nano-banana-pro'].map((v) => ({ value: v }))}
                onChange={(v) => set({ iconModel: v })}
                placeholder="图像模型"
              />
              {smallCount > 0 && (
                <Checkbox checked={value.iconForceSmall} onChange={(e) => set({ iconForceSmall: e.target.checked })}>
                  也重绘 {smallCount} 个过小的图标（原图小于 {MIN_REDRAW_PX}px，容易画错）
                </Checkbox>
              )}
              {p.apiReady ? (
                <Button block icon={<ThunderboltOutlined />} loading={busy === 'icons'} disabled={!redrawIds.length || redrawIds.length > 16} onClick={() => p.onGenerateIcons(value.iconForceSmall ? redrawIds : null)}>
                  {redrawIds.length ? `AI 重绘高清图标（${redrawIds.length} 个，消耗 1 次生成）` : '没有可重绘的图标'}
                </Button>
              ) : (
                <Button block onClick={p.onOpenSettings}>
                  先在设置里填写 API Key
                </Button>
              )}
              {unnamed.length > 0 && (
                <div className="panel-hint block warn">
                  {unnamed.length} 个图标还没有名称：{unnamed.slice(0, 6).join('、')}
                  {unnamed.length > 6 ? ' 等' : ''}。模型只能看图猜含义，点击图标在上方填写名称（如 calendar、store）或说明后再重绘更准确。
                </div>
              )}
              <div className="panel-hint block">
                所有图标画在一张网格图上一次重绘，风格统一；会同时把完整设计稿（框出图标位置）作为参考，让模型结合旁边的文字理解含义。完成后自动拆开、去白底，替换原来的裁剪素材。{redrawIds.length > 16 && '一次最多 16 个。'}
              </div>
            </>
          )}
          {state?.result?.dir && (
            <div className="ds-folder">
              <Button block icon={<FolderOpenOutlined />} onClick={() => p.onOpenFolder(state.result!.dir)}>
                打开解析文件夹
              </Button>
              <Tooltip title="spec（.spec.json）、编号标注图（.spec.png）和图标素材（icons/）都在这个文件夹里">
                <button type="button" className="ds-paths link-like" onClick={() => p.onOpenFolder(state.result!.dir)}>
                  {state.result.dir}
                </button>
              </Tooltip>
            </div>
          )}
        </>
      )}
      {!spec && (
        <div className="panel-hint block">
          把 UI 设计稿或一个好看的样式（卡片、按钮……）解析成结构化描述：节点坐标、类型、颜色、圆角、描边、阴影、字号与自动布局，供 Agent 还原代码。语义可以在这里标注，也可以交给 Agent。
        </div>
      )}
    </div>
  );
}
