#!/usr/bin/env node
// MCP 端到端冒烟测试：模拟 Agent 客户端（声明 roots 能力），在临时项目里调用全部工具。
// 用法：node scripts/mcp-smoke.mjs [imgtool 可执行文件路径]
import { spawn } from 'node:child_process';
import http from 'node:http';
import { mkdtempSync, mkdirSync, writeFileSync, existsSync, readFileSync, statSync, realpathSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve as resolvePath } from 'node:path';
import { deflateSync } from 'node:zlib';
import { pathToFileURL } from 'node:url';

const bin = resolvePath(process.argv[2] ?? 'target/debug/imgtool');

// ---- 生成带噪声的 PNG（无第三方依赖） ----
const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc32 = (buf) => {
  let c = 0xffffffff;
  for (const b of buf) c = crcTable[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};
const chunk = (type, data) => {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const td = Buffer.concat([Buffer.from(type), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(td));
  return Buffer.concat([len, td, crc]);
};
function makePng(w, h) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 2; // RGB
  const raw = Buffer.alloc((w * 3 + 1) * h);
  let seed = 7;
  for (let y = 0; y < h; y++) {
    raw[y * (w * 3 + 1)] = 0;
    for (let x = 0; x < w; x++) {
      seed = (seed * 1103515245 + 12345) >>> 0;
      const n = (seed >>> 16) & 0x3f;
      const o = y * (w * 3 + 1) + 1 + x * 3;
      raw[o] = ((x * 255) / w) ^ n;
      raw[o + 1] = (y * 255) / h;
      raw[o + 2] = n * 3;
    }
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw)),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

/** 白底 1024x1024，中间 400x400 红色方块：模拟"纯白背景"的生成结果 */
function makeWhiteCard() {
  const w = 1024, h = 1024;
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8;
  ihdr[9] = 2;
  const raw = Buffer.alloc((w * 3 + 1) * h, 255);
  for (let y = 0; y < h; y++) {
    raw[y * (w * 3 + 1)] = 0;
    for (let x = 312; x < 712; x++) {
      if (y >= 312 && y < 712) {
        const o = y * (w * 3 + 1) + 1 + x * 3;
        raw[o] = 220; raw[o + 1] = 40; raw[o + 2] = 40;
      }
    }
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw)),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

/** 白底 300x100，3 个 50x50 深色方块 */
function makeSheet() {
  const w = 300, h = 100;
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8;
  ihdr[9] = 2;
  const raw = Buffer.alloc((w * 3 + 1) * h, 255);
  for (let y = 0; y < h; y++) {
    raw[y * (w * 3 + 1)] = 0;
    for (let x = 0; x < w; x++) {
      if (y >= 25 && y < 75 && x % 100 >= 25 && x % 100 < 75) {
        const o = y * (w * 3 + 1) + 1 + x * 3;
        raw[o] = 30; raw[o + 1] = 30; raw[o + 2] = 30;
      }
    }
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw)),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

// ---- 准备临时项目 ----
const project = realpathSync(mkdtempSync(join(tmpdir(), 'imgtool-smoke-')));
mkdirSync(join(project, 'public/img'), { recursive: true });
mkdirSync(join(project, 'dist'), { recursive: true });
writeFileSync(join(project, '.gitignore'), 'dist/\n');
writeFileSync(join(project, 'public/img/hero.png'), makePng(1600, 1000));
writeFileSync(join(project, 'public/img/icon.png'), makePng(64, 64));
writeFileSync(join(project, 'dist/ignored.png'), makePng(64, 64));
writeFileSync(join(tmpdir(), 'outside.png'), makePng(8, 8));

// ---- JSON-RPC over stdio ----
// 不在项目目录启动，验证根目录确实来自客户端 Roots
// ---- 假的 hfsyapi：图床上传 / 创建任务 / 查询任务 / 下载结果 ----
const fake = { uploads: 0, creates: [], queries: 0 };
const fakeServer = http.createServer((req, res) => {
  let body = '';
  req.on('data', (c) => (body += c));
  req.on('end', () => {
    const json = (code, obj) => {
      res.writeHead(code, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify(obj));
    };
    // 结果图链接是公开地址，下载时不带 Key
    if (req.method === 'GET' && req.url.startsWith('/files/')) {
      res.writeHead(200, { 'Content-Type': 'image/png' });
      return res.end(makeWhiteCard());
    }
    if (req.headers.authorization !== 'Bearer sk-fake') return json(401, { message: 'Invalid token', success: false });
    const base = `http://127.0.0.1:${fakeServer.address().port}`;
    if (req.method === 'POST' && req.url === '/v1/files/image-upload') {
      fake.uploads++;
      return json(200, { success: true, message: '', data: { url: `${base}/files/ref.png?Expires=4102444800`, size: 1 } });
    }
    if (req.method === 'POST' && req.url === '/v1/image/create') {
      fake.creates.push(JSON.parse(body));
      return json(200, { id: `task_${fake.creates.length}`, status: 'queued', images: [], error: null });
    }
    if (req.method === 'GET' && req.url.startsWith('/v1/image/query/')) {
      fake.queries++;
      // 第一次查询仍在生成，之后完成
      if (fake.queries % 2 === 1) return json(200, { id: 'x', status: 'in_progress', images: [], error: null });
      return json(200, { id: 'x', status: 'completed', images: [{ url: `${base}/files/out.png`, revised_prompt: 'rp' }], error: null });
    }
    json(404, { message: 'not found' });
  });
});
await new Promise((r) => fakeServer.listen(0, '127.0.0.1', r));
const fakeBase = `http://127.0.0.1:${fakeServer.address().port}`;

const child = spawn(bin, ['mcp'], {
  cwd: tmpdir(),
  stdio: ['pipe', 'pipe', 'inherit'],
  env: { ...process.env, IMGTOOL_API_BASE_URL: fakeBase, IMGTOOL_API_KEY: 'sk-fake', IMGTOOL_REF_UPLOAD: '1' },
});
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
      send({ jsonrpc: '2.0', id: msg.id, result: { roots: [{ uri: pathToFileURL(project).href, name: 'smoke' }] } });
    } else if (msg.id !== undefined && pending.has(msg.id)) {
      pending.get(msg.id)(msg);
      pending.delete(msg.id);
    }
  }
});
const send = (m) => child.stdin.write(JSON.stringify(m) + '\n');
const request = (method, params) =>
  new Promise((resolve) => {
    const id = nextId++;
    pending.set(id, resolve);
    send({ jsonrpc: '2.0', id, method, params });
  });
const call = async (name, args = {}) => {
  const res = await request('tools/call', { name, arguments: args });
  if (res.error) throw new Error(`${name}: ${JSON.stringify(res.error)}`);
  const [summary, json] = res.result.content;
  return { isError: !!res.result.isError, summary: summary.text, data: json ? JSON.parse(json.text) : null };
};

let failures = 0;
const check = (cond, label) => {
  console.log(`${cond ? 'PASS' : 'FAIL'}  ${label}`);
  if (!cond) failures++;
};

try {
  const init = await request('initialize', {
    protocolVersion: '2025-06-18',
    capabilities: { roots: { listChanged: true } },
    clientInfo: { name: 'smoke', version: '0' },
  });
  check(init.result?.serverInfo?.name === 'imgtool', 'initialize 返回 serverInfo');
  send({ jsonrpc: '2.0', method: 'notifications/initialized' });

  const tools = await request('tools/list', {});
  const names = tools.result.tools.map((t) => t.name).sort();
  console.log('      tools:', names.join(', '));
  check(names.length === 14, '暴露 14 个工具');

  const ctx = await call('get_project_context');
  check(ctx.data.roots[0] === project, `根目录来自客户端 Roots: ${ctx.data.roots[0]}`);

  const init2 = await call('init_project_preset');
  check(!init2.isError && existsSync(join(project, '.imgtool.toml')), 'init_project_preset 创建预设');

  const scan = await call('scan_images', {});
  check(scan.data.total_images === 2, `scan 遵守 .gitignore（${scan.data.total_images} 张）`);
  check(scan.data.images[0].path === 'public/img/hero.png', 'scan 返回相对路径并按体积排序');

  const dry = await call('compress_images', { paths: ['public/img/hero.png'], max_kb: 100, dry_run: true });
  check(dry.data.written === 0 && !existsSync(join(project, 'public/img/hero.min.png')), 'dry_run 不写文件');

  const comp = await call('compress_images', { paths: ['public/img/hero.png'], max_kb: 100, max_width: 1200 });
  const out = join(project, 'public/img/hero.min.png');
  check(existsSync(out) && statSync(out).size <= 100 * 1024, `目标体积压缩: ${comp.summary}`);
  check(comp.data.items[0].width <= 1200, `同时限制分辨率（${comp.data.items[0].width}px）`);

  const again = await call('compress_images', { paths: ['public/img/hero.png'], max_kb: 100 });
  check(again.data.failed === 1, '目标已存在时默认报错，不覆盖');

  const conv = await call('convert_images', { paths: ['public/img'], format: 'webp', quality: 75 });
  check(existsSync(join(project, 'public/img/hero.webp')) && existsSync(join(project, 'public/img/icon.webp')), `目录批量转 webp: ${conv.summary}`);
  check(conv.data.processed === 2, '目录展开时跳过已生成的 .min 产物');

  const crop = await call('crop_image', { path: 'public/img/hero.png', aspect: '1:1', resize_width: 256, output: 'public/img/avatar.avif', format: 'avif' });
  check(!crop.isError && existsSync(join(project, 'public/img/avatar.avif')), `裁剪 1:1 并转 avif: ${crop.summary}`);
  check(crop.data.items?.[0]?.width === 256 && crop.data.items?.[0]?.height === 256, '裁剪后尺寸 256x256');

  const outside = await call('compress_images', { paths: [join(tmpdir(), 'outside.png')] });
  check(outside.isError, `拒绝项目外路径: ${outside.summary}`);
  const traversal = await call('crop_image', { path: 'public/img/icon.png', aspect: '1:1', output: '../evil.png' });
  check(traversal.isError, '拒绝 .. 输出路径');

  const bg = await call('remove_background', { paths: ['public/img/icon.png'], output_mode: 'dir', out_dir: 'nobg' });
  check(!bg.isError && existsSync(join(project, 'nobg/icon.png')), `remove_background 输出 png: ${bg.summary}`);
  const bgJpeg = await call('remove_background', { paths: ['public/img/icon.png'], format: 'jpeg' });
  check(bgJpeg.isError, 'remove_background 拒绝 jpeg 输出');

  // 智能裁剪：白底上 3 个深色方块
  writeFileSync(join(project, 'public/img/sheet.png'), makeSheet());
  const det = await call('detect_icons', { path: 'public/img/sheet.png' });
  check(det.data?.result?.regions?.length === 3, `detect_icons 识别 3 个: ${det.summary}`);
  const split = await call('split_icons', { path: 'public/img/sheet.png', names: ['home', 'discover', 'me'], size: 128 });
  check(!split.isError && existsSync(join(project, 'public/img/sheet_icons/discover.png')), `split_icons 按名字导出: ${split.summary}`);
  const splitOutside = await call('split_icons', { path: 'public/img/sheet.png', out_dir: '../escape' });
  check(splitOutside.isError, 'split_icons 拒绝项目外输出目录');

  // ---- Agent 设计稿工作流：导入对话图片 → 查看 → 生成（参考图 + 最终尺寸 + 去背景）→ 查询结果 ----
  const ctx2 = await call('get_project_context');
  check(ctx2.data.generation?.configured === true && !JSON.stringify(ctx2.data).includes('sk-fake'), 'get_project_context 显示生成已配置且不泄露 Key');

  const designB64 = makeSheet().toString('base64');
  const imp = await call('import_image', { base64: `data:image/png;base64,${designB64}`, filename: '首页设计稿.jpg' });
  check(!imp.isError && imp.data.path === '.imgtool/inbox/首页设计稿.png', `import_image(base64) 按实际格式命名: ${imp.summary}`);
  const impUrl = await call('import_image', { url: `${fakeBase}/files/out.png` });
  check(!impUrl.isError && impUrl.data.path === '.imgtool/inbox/out.png' && impUrl.data.info.width === 1024, `import_image(url): ${impUrl.summary}`);
  const impPath = await call('import_image', { source_path: join(tmpdir(), 'outside.png') });
  check(!impPath.isError && impPath.data.path.startsWith('.imgtool/inbox/outside'), `import_image(项目外文件) 复制进项目: ${impPath.summary}`);
  const impBad = await call('import_image', { base64: Buffer.from('hello').toString('base64') });
  check(impBad.isError, '拒绝非图片数据');
  const impTwo = await call('import_image', { url: 'https://x', base64: 'aaaa' });
  check(impTwo.isError, '多个来源同时给出时报错');

  const viewRes = await request('tools/call', { name: 'view_image', arguments: { path: imp.data.path, max_size: 128 } });
  const imgBlock = viewRes.result.content.find((c) => c.type === 'image');
  check(!!imgBlock && imgBlock.mimeType.startsWith('image/') && imgBlock.data.length > 50, 'view_image 返回缩略图');

  const gen = await call('generate_image', {
    prompt: '按参考图重新设计，纯白背景',
    reference_images: [imp.data.path],
    final_size: '300x200',
    remove_background: true,
    name: 'redesign',
  });
  check(!gen.isError && gen.data.task_id?.startsWith('gen_') && gen.data.size === '1008x672', `generate_image 立即返回任务（按 300x200 自动选 3:2→实际 ${gen.data.size}）: ${gen.summary}`);
  let st;
  for (let i = 0; i < 5; i++) {
    const r = await request('tools/call', { name: 'get_generation', arguments: { task_id: gen.data.task_id, wait_seconds: 20 } });
    st = { json: JSON.parse(r.result.content[1].text), previews: r.result.content.filter((c) => c.type === 'image').length };
    if (st.json.status !== 'running') break;
  }
  const out0 = st.json.files?.[0];
  check(st.json.status === 'completed' && out0?.path.startsWith('.imgtool/generated/redesign-') && out0.width === 300 && out0.height === 200, `get_generation 完成并调整到 300x200: ${JSON.stringify(st.json.files)}`);
  check(st.previews === 1, 'get_generation 附带结果缩略图');
  check(fake.uploads === 1 && fake.creates[0]?.reference_images?.[0]?.startsWith(fakeBase), '本地参考图先上传图床再作为地址传给生成接口');
  const alpha = readFileSync(join(project, out0.path)).includes(Buffer.from('tRNS')) || readFileSync(join(project, out0.path))[25] === 6;
  check(alpha, '生成后去除背景，输出带透明通道的 PNG');
  const badRef = await call('generate_image', { prompt: 'x', reference_images: ['/etc/passwd'] });
  check(badRef.isError, '拒绝项目外的参考图路径');
  const noTask = await call('get_generation', { task_id: 'gen_nope', wait_seconds: 0 });
  check(noTask.isError, '查询不存在的任务时报错');

  const info = await call('image_info', { paths: ['public/img/hero.webp'] });
  check(info.data[0].info.format === 'webp', 'image_info 识别格式');

  check(readFileSync(join(project, 'public/img/hero.png')).length > 0, '原图保留');
} catch (e) {
  console.error(e);
  failures++;
} finally {
  child.kill();
}
console.log(failures ? `\n${failures} 项失败` : '\n全部通过');
process.exit(failures ? 1 : 0);
