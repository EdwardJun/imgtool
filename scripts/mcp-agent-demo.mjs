#!/usr/bin/env node
// 真实接口的 Agent 工作流演示：像 Agent 一样通过 MCP 调用 imgtool，
// 导入"对话里收到的设计稿" → 按规则重新生成 → 去背景 → 切图命名 → 压缩 → 回看结果。
// 会消耗一次真实生成额度。Key 只从环境变量读取：
//   IMGTOOL_API_KEY=... [IMGTOOL_API_BASE_URL=...] node scripts/mcp-agent-demo.mjs <设计稿图片> [imgtool 路径]
import { spawn } from 'node:child_process';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { resolve as resolvePath, join, basename, extname } from 'node:path';
import { pathToFileURL } from 'node:url';

const design = process.argv[2];
if (!design) {
  console.error('用法: node scripts/mcp-agent-demo.mjs <设计稿图片> [imgtool 路径]');
  process.exit(2);
}
const bin = resolvePath(process.argv[3] ?? 'target/debug/imgtool');
const project = process.cwd();
const outDir = '.imgtool/agent-demo';
mkdirSync(join(project, outDir), { recursive: true });

const child = spawn(bin, ['mcp'], { cwd: project, stdio: ['pipe', 'pipe', 'ignore'] });
let buffer = '';
let nextId = 1;
const pending = new Map();
child.stdout.on('data', (d) => {
  buffer += d;
  let idx;
  while ((idx = buffer.indexOf('\n')) >= 0) {
    const line = buffer.slice(0, idx).trim();
    buffer = buffer.slice(idx + 1);
    if (!line) continue;
    const msg = JSON.parse(line);
    if (msg.method === 'roots/list') {
      send({ jsonrpc: '2.0', id: msg.id, result: { roots: [{ uri: pathToFileURL(project).href, name: 'demo' }] } });
    } else if (msg.id !== undefined && pending.has(msg.id)) {
      pending.get(msg.id)(msg);
      pending.delete(msg.id);
    }
  }
});
const send = (m) => child.stdin.write(JSON.stringify(m) + '\n');
const request = (method, params) =>
  new Promise((res) => {
    const id = nextId++;
    pending.set(id, res);
    send({ jsonrpc: '2.0', id, method, params });
  });

const t0 = Date.now();
const elapsed = () => `${((Date.now() - t0) / 1000).toFixed(1)}s`;
/** 调用工具并打印 Agent 能看到的内容（文本摘要 + 图片块数量） */
async function call(name, args = {}) {
  const started = Date.now();
  const res = await request('tools/call', { name, arguments: args });
  if (res.error) throw new Error(`${name}: ${JSON.stringify(res.error)}`);
  const content = res.result.content;
  const texts = content.filter((c) => c.type === 'text');
  const images = content.filter((c) => c.type === 'image');
  const took = ((Date.now() - started) / 1000).toFixed(1);
  console.log(`\n[${elapsed()}] ${name}（${took}s）${res.result.isError ? ' ✗' : ''}`);
  console.log('  ' + texts[0]?.text);
  if (images.length) console.log(`  + ${images.length} 张图片块（Agent 可直接看到）`);
  if (res.result.isError) throw new Error(texts[0]?.text);
  return { data: texts[1] ? JSON.parse(texts[1].text) : null, images };
}
/** 把 Agent 看到的缩略图存下来，便于人工核对 */
const saveBlocks = (images, prefix) =>
  images.forEach((img, i) => {
    const ext = img.mimeType.split('/')[1] === 'jpeg' ? 'jpg' : img.mimeType.split('/')[1];
    writeFileSync(join(project, outDir, `${prefix}-${i}.${ext}`), Buffer.from(img.data, 'base64'));
  });

// Agent 收到的用户规则
const RULES = `参考这张 App 首页设计稿底部导航栏的 4 个图标（首页、发现、购物车、我的），重新设计一套图标：
- 4 个图标从左到右横向等距排列，顺序不变，彼此留出足够间距
- 扁平线性风格，2px 圆角描边，主色 #4F46E5（靛蓝），不要渐变和阴影
- 纯白背景，图标下方不要任何文字
- 每个图标造型简洁，语义清晰：房子、指南针、购物车、人像`;

try {
  await request('initialize', {
    protocolVersion: '2025-06-18',
    capabilities: { roots: { listChanged: true } },
    clientInfo: { name: 'agent-demo', version: '0' },
  });
  send({ jsonrpc: '2.0', method: 'notifications/initialized' });

  const ctx = await call('get_project_context');
  if (!ctx.data.generation?.configured) throw new Error('生成接口未配置（设置 IMGTOOL_API_KEY 或在桌面端保存设置）');

  // 1. 对话里的图片以 base64 形式到达
  const b64 = readFileSync(design).toString('base64');
  const name = basename(design, extname(design));
  const imp = await call('import_image', { base64: b64, filename: `${name}-收到`, dir: outDir });

  // 2. Agent 先看一眼原图
  const view = await call('view_image', { path: imp.data.path, max_size: 512 });
  saveBlocks(view.images, 'seen-input');

  // 3. 按规则生成（参考图 + 去背景）
  const gen = await call('generate_image', {
    prompt: RULES,
    reference_images: [imp.data.path],
    ratio: '3:1',
    remove_background: true,
    out_dir: outDir,
    name: 'navbar-redesign',
  });

  // 4. 轮询结果（每次最多等 55s）
  let st;
  for (let i = 0; i < 10; i++) {
    st = await call('get_generation', { task_id: gen.data.task_id, wait_seconds: 55 });
    if (st.data.status !== 'running') break;
  }
  if (st.data.status !== 'completed') throw new Error(`生成未完成: ${st.data.status} ${st.data.error ?? ''}`);
  saveBlocks(st.images, 'seen-generated');
  const genPath = st.data.files[0].path;

  // 5. 切出 4 个图标并语义化命名
  const det = await call('detect_icons', { path: genPath });
  const names = ['home', 'discover', 'cart', 'me'];
  const split = await call('split_icons', {
    path: genPath,
    names: names.slice(0, det.data.result.regions.length),
    size: 128,
    out_dir: `${outDir}/icons`,
    on_conflict: 'overwrite',
  });

  // 6. 交付前压缩为 webp
  await call('convert_images', { paths: [`${outDir}/icons`], format: 'webp', quality: 85, on_conflict: 'overwrite' });

  // 7. 回看其中一个图标
  const v2 = await call('view_image', { path: split.data.files?.[0]?.path ?? `${outDir}/icons/home.png`, max_size: 128 });
  saveBlocks(v2.images, 'seen-icon');

  console.log(`\n完成，总耗时 ${elapsed()}。产物目录：${outDir}`);
} catch (e) {
  console.error('\n失败:', e.message);
  process.exitCode = 1;
} finally {
  child.kill();
}
