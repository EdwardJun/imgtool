import type { Bounds, SpecNode } from './types';

/** 没有样式、也没有语义的自动分组 */
export const isPlainGroup = (n: SpecNode) =>
  n.type === 'frame' && !n.style?.fill && !n.style?.stroke && !n.style?.shadow && n.source === 'auto' && !n.role && !n.name && !n.text && !n.description;

export interface FlatNode {
  node: SpecNode;
  depth: number;
  parentId: string | null;
}

/** 深度优先展开（父节点在前） */
export function flatten(root: SpecNode): FlatNode[] {
  const out: FlatNode[] = [];
  const walk = (n: SpecNode, depth: number, parentId: string | null) => {
    out.push({ node: n, depth, parentId });
    for (const c of n.children ?? []) walk(c, depth + 1, n.id);
  };
  walk(root, 0, null);
  return out;
}

export function findNode(root: SpecNode, id: string): SpecNode | null {
  if (root.id === id) return root;
  for (const c of root.children ?? []) {
    const f = findNode(c, id);
    if (f) return f;
  }
  return null;
}

/** 不可变更新一个节点 */
export function updateNode(root: SpecNode, id: string, patch: (n: SpecNode) => SpecNode): SpecNode {
  if (root.id === id) return patch(root);
  if (!root.children?.length) return root;
  let changed = false;
  const children = root.children.map((c) => {
    const next = updateNode(c, id, patch);
    if (next !== c) changed = true;
    return next;
  });
  return changed ? { ...root, children } : root;
}

/** 删除节点；它的子节点上移到父节点下，避免误删整棵子树。根节点不能删 */
export function removeNode(root: SpecNode, id: string): SpecNode {
  if (!root.children?.length) return root;
  const idx = root.children.findIndex((c) => c.id === id);
  if (idx >= 0) {
    const target = root.children[idx];
    const children = [...root.children.slice(0, idx), ...(target.children ?? []), ...root.children.slice(idx + 1)];
    return { ...root, children };
  }
  let changed = false;
  const children = root.children.map((c) => {
    const next = removeNode(c, id);
    if (next !== c) changed = true;
    return next;
  });
  return changed ? { ...root, children } : root;
}

export function maxId(root: SpecNode): number {
  return flatten(root).reduce((m, f) => Math.max(m, Number(f.node.id.replace(/^n/, '')) || 0), 0);
}

/** 新增一个框：挂在根节点下（「整理层级」后会放进包含它的容器） */
export function addNode(root: SpecNode, bounds: Bounds): { root: SpecNode; id: string } {
  const id = `n${maxId(root) + 1}`;
  const node: SpecNode = {
    id,
    type: 'rect',
    bounds,
    relative: { x: bounds.x - root.bounds.x, y: bounds.y - root.bounds.y },
    confidence: 1,
    source: 'manual',
  };
  return { root: { ...root, children: [...(root.children ?? []), node] }, id };
}

/** 局部重新分析得到的样式合并进节点：保留编号、语义、框和子节点 */
export function mergeAnalyzed(node: SpecNode, fresh: SpecNode): SpecNode {
  const keepType = node.source !== 'auto' || (node.children?.length ?? 0) > 0;
  const type = keepType ? node.type : fresh.children?.length ? 'rect' : fresh.type;
  return { ...node, type, style: fresh.style, text_style: fresh.text_style ?? (type === 'text' ? node.text_style : undefined), source: 'manual' };
}

export const nodeLabel = (n: SpecNode) => {
  const kind = isPlainGroup(n) ? 'group' : n.type;
  const name = n.name || n.text || n.role;
  return `${n.id.replace(/^n/, '')} · ${kind}${name ? ` · ${name}` : ''}`;
};
