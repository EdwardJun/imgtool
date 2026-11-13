#!/usr/bin/env node
// MCP 端到端冒烟测试：模拟 Agent 客户端（声明 roots 能力），在临时项目里调用全部工具。
// 用法：node scripts/mcp-smoke.mjs [imgtool 可执行文件路径]
import { spawn } from 'node:child_process';
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
const child = spawn(bin, ['mcp'], { cwd: tmpdir(), stdio: ['pipe', 'pipe', 'inherit'] });
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
  check(names.length === 7, '暴露 7 个工具');

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
