// 仅开发环境：在普通浏览器里打开前端时模拟 Tauri 后端，方便调界面。
// 用 canvas 生成示例图，预览用 canvas.toBlob 真实编码，体积数据是真实的。
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks';

const blobs = new Map<string, { url: string; width: number; height: number; bytes: number; canvas: HTMLCanvasElement }>();

function makeSample(name: string, width: number, height: number, hue: number) {
  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext('2d')!;
  const g = ctx.createLinearGradient(0, 0, width, height);
  g.addColorStop(0, `hsl(${hue}, 70%, 55%)`);
  g.addColorStop(1, `hsl(${hue + 60}, 70%, 35%)`);
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, width, height);
  for (let i = 0; i < 400; i++) {
    ctx.fillStyle = `hsla(${(hue + i * 7) % 360}, 80%, ${30 + (i % 50)}%, 0.5)`;
    ctx.beginPath();
    ctx.arc(Math.random() * width, Math.random() * height, Math.random() * 40 + 4, 0, Math.PI * 2);
    ctx.fill();
  }
  ctx.fillStyle = '#fff';
  ctx.font = `bold ${Math.round(width / 10)}px sans-serif`;
  ctx.fillText(name, width * 0.08, height * 0.55);
  return canvas;
}

/** 白底产品图：用来调试去除背景 */
function makeProduct(width: number, height: number) {
  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext('2d')!;
  ctx.fillStyle = '#fdfdfd';
  ctx.fillRect(0, 0, width, height);
  ctx.fillStyle = '#d42a2a';
  ctx.beginPath();
  ctx.roundRect(width * 0.2, height * 0.25, width * 0.6, height * 0.5, 40);
  ctx.fill();
  ctx.fillStyle = '#fdfdfd';
  ctx.beginPath();
  ctx.arc(width * 0.62, height * 0.5, height * 0.14, 0, Math.PI * 2);
  ctx.fill();
  ctx.fillStyle = '#222';
  ctx.fillRect(width * 0.28, height * 0.35, width * 0.18, height * 0.3);
  return canvas;
}

/** 透明底导航栏：5 个线性图标 + 中文标签，用来调试智能裁剪 */
function makeNav(width: number, height: number) {
  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext('2d')!;
  ctx.strokeStyle = '#ddd';
  ctx.fillStyle = '#ddd';
  ctx.lineWidth = 7;
  ctx.font = '30px "PingFang SC", sans-serif';
  ctx.textAlign = 'center';
  const labels = ['首页', '发现', '发布', '消息', '我的'];
  labels.forEach((label, i) => {
    const cx = 160 + i * 220;
    const cy = 200;
    ctx.beginPath();
    if (i === 1) ctx.arc(cx, cy, 40, 0, Math.PI * 2);
    else ctx.roundRect(cx - 40, cy - 40, 80, 80, i === 3 ? 30 : 12);
    ctx.stroke();
    if (i === 3) for (let d = -1; d <= 1; d++) ctx.fillRect(cx + d * 20 - 4, cy - 4, 8, 8);
    if (i === 2) {
      ctx.fillRect(cx - 3, cy - 22, 6, 44);
      ctx.fillRect(cx - 22, cy - 3, 44, 6);
    }
    ctx.fillText(label, cx, cy + 90);
  });
  return canvas;
}

/** 浏览器 Mock 的简化识别：按列投影切分，只用于看界面 */
function mockDetect(canvas: HTMLCanvasElement, gap: number) {
  const ctx = canvas.getContext('2d')!;
  const { data, width, height } = ctx.getImageData(0, 0, canvas.width, canvas.height);
  const fg = (x: number, y: number) => {
    const i = (y * width + x) * 4;
    return data[i + 3] > 32 && (data[i] < 240 || data[i + 3] < 255);
  };
  const cols: [number, number][] = [];
  for (let x = 0; x < width; x++) {
    let y0 = -1;
    let y1 = -1;
    for (let y = 0; y < height; y++) if (fg(x, y)) (y0 < 0 && (y0 = y), (y1 = y));
    cols.push([y0, y1]);
  }
  const regions = [];
  let start = -1;
  let last = -1;
  for (let x = 0; x <= width; x++) {
    const on = x < width && cols[x][0] >= 0;
    if (on) {
      if (start < 0) start = x;
      last = x;
    } else if (start >= 0 && x - last > gap) {
      const ys = cols.slice(start, last + 1).filter((c) => c[0] >= 0);
      const y0 = Math.min(...ys.map((c) => c[0]));
      const y1 = Math.max(...ys.map((c) => c[1]));
      regions.push({ x: start, y: y0, width: last - start + 1, height: y1 - y0 + 1, kind: 'icon_with_label' });
      start = -1;
    }
  }
  return regions;
}

/** 粗略模拟去除背景：只做全图阈值，用来看界面效果 */
function mockRemoveBg(src: HTMLCanvasElement, r: { color?: number[] | null; tolerance: number; softness: number }) {
  const c = document.createElement('canvas');
  c.width = src.width;
  c.height = src.height;
  const ctx = c.getContext('2d')!;
  ctx.drawImage(src, 0, 0);
  const data = ctx.getImageData(0, 0, c.width, c.height);
  const bg = r.color ?? [data.data[0], data.data[1], data.data[2]];
  const tol = r.tolerance;
  const limit = tol + r.softness;
  for (let i = 0; i < data.data.length; i += 4) {
    const d = Math.hypot(data.data[i] - bg[0], data.data[i + 1] - bg[1], data.data[i + 2] - bg[2]) / 441.67 * 100;
    const w = d <= tol ? 1 : d < limit ? 1 - (d - tol) / (limit - tol) : 0;
    data.data[i + 3] = Math.round(data.data[i + 3] * (1 - w));
  }
  ctx.putImageData(data, 0, 0);
  return { canvas: c, color: bg as [number, number, number] };
}

async function toBlob(canvas: HTMLCanvasElement, type: string, quality: number) {
  return new Promise<Blob>((resolve) => canvas.toBlob((b) => resolve(b!), type, quality));
}

async function register(path: string, canvas: HTMLCanvasElement) {
  const blob = await toBlob(canvas, 'image/png', 1);
  const url = URL.createObjectURL(blob);
  blobs.set(path, { url, width: canvas.width, height: canvas.height, bytes: blob.size, canvas });
}

const MIME: Record<string, string> = { jpeg: 'image/jpeg', png: 'image/png', webp: 'image/webp', avif: 'image/webp' };

export async function install() {
  await register('/demo/photo.png', makeSample('photo', 1600, 1000, 200));
  await register('/demo/square.png', makeSample('square', 800, 800, 20));
  await register('/demo/tall.png', makeSample('tall', 600, 1000, 300));
  await register('/demo/product.png', makeProduct(900, 700));
  await register('/demo/nav.png', makeNav(1200, 500));

  mockIPC(
    async (cmd, args) => {
      const a = (args ?? {}) as Record<string, any>;
      switch (cmd) {
        case 'initial_paths':
          return [...blobs.keys()];
        case 'expand_paths':
          return a.paths;
        case 'load_images':
          return a.paths.map((p: string) => {
            const b = blobs.get(p);
            if (!b) return { Err: `${p}: 不存在` };
            return {
              Ok: {
                path: p,
                name: p.split('/').pop(),
                info: { format: 'png', width: b.width, height: b.height, bytes: b.bytes, decodable: true },
                display_path: b.url,
              },
            };
          });
        case 'preview': {
          const b = blobs.get(a.path)!;
          const o = a.options;
          let src: HTMLCanvasElement = b.canvas;
          let background = null;
          if (o.remove_background) {
            const r = mockRemoveBg(src, o.remove_background);
            src = r.canvas;
            background = { color: r.color, border_match: 1, removed_ratio: 0.5 };
          }
          if (o.crop?.type === 'rect') {
            const c = document.createElement('canvas');
            c.width = o.crop.width;
            c.height = o.crop.height;
            c.getContext('2d')!.drawImage(src, o.crop.x, o.crop.y, o.crop.width, o.crop.height, 0, 0, c.width, c.height);
            src = c;
          }
          if (o.resize?.width || o.resize?.height) {
            const scale = o.resize.width ? o.resize.width / src.width : o.resize.height / src.height;
            const c = document.createElement('canvas');
            c.width = Math.round(src.width * Math.min(scale, 1));
            c.height = Math.round(src.height * Math.min(scale, 1));
            c.getContext('2d')!.drawImage(src, 0, 0, c.width, c.height);
            src = c;
          }
          const format = o.format ?? 'png';
          const q = o.mode.type === 'quality' ? o.mode.quality / 100 : 0.6;
          const blob = await toBlob(src, MIME[format], q);
          return {
            output_path: URL.createObjectURL(blob),
            report: {
              format,
              width: src.width,
              height: src.height,
              original_format: 'png',
              original_width: b.width,
              original_height: b.height,
              original_bytes: b.bytes,
              output_bytes: blob.size,
              quality: o.mode.type === 'quality' ? o.mode.quality : 60,
              kept_original: false,
              target_met: o.mode.type === 'target_size' ? blob.size <= o.mode.max_bytes : null,
              background,
              warnings: format === 'avif' ? ['浏览器 Mock 模式用 WebP 代替 AVIF'] : [],
            },
          };
        }
        case 'find_preset':
          return { path: '/demo/.imgtool.toml', options: { mode: { type: 'target_size', max_bytes: 300 * 1024 }, resize: { width: 1920, height: 1920, fit: 'contain', without_enlargement: true }, format: 'webp', keep_original_if_larger: true } };
        case 'export_images':
          return { total: a.items.length, succeeded: a.items.length, failed: 0, original_bytes: 1, output_bytes: 1 };
        case 'mcp_status':
          return {
            command: '/Applications/ImgTool.app/Contents/MacOS/imgtool',
            error: null,
            snippet: JSON.stringify({ mcpServers: { imgtool: { command: '/Applications/ImgTool.app/Contents/MacOS/imgtool', args: ['mcp'] } } }, null, 2),
            clients: [
              { id: 'claude_code', name: 'Claude Code', config_path: '~/.claude.json', configured: true, up_to_date: true },
              { id: 'cursor', name: 'Cursor', config_path: '~/.cursor/mcp.json', configured: false, up_to_date: false },
              { id: 'kiro', name: 'Kiro', config_path: '~/.kiro/settings/mcp.json', configured: true, up_to_date: false },
            ],
          };
        case 'mcp_install':
          return '~/.cursor/mcp.json';
        case 'get_api_settings':
          return JSON.parse(localStorage.getItem('mock.api') ?? '{"base_url":"","api_key":"","remember":false}');
        case 'save_api_settings':
          localStorage.setItem('mock.api', JSON.stringify(a.settings));
          return null;
        case 'generate_image': {
          const p = a.params;
          await new Promise((r) => setTimeout(r, 1500));
          if (!p.api_key.startsWith('sk-')) throw '生成服务调用失败: 创建任务失败: HTTP 401 Unauthorized Invalid token（请检查 API Key）';
          const canvas = makeSample(p.model, Math.round(p.width / 4), Math.round(p.height / 4), (p.prompt.length * 37) % 360);
          const blob = await toBlob(canvas, 'image/png', 1);
          return {
            task_id: 'task_mock',
            path: URL.createObjectURL(blob),
            format: p.format,
            width: p.width,
            height: p.height,
            bytes: blob.size,
            source_url: null,
            revised_prompt: p.prompt,
            warnings: p.remove_background ? ['浏览器 Mock 模式不会真正去除背景'] : [],
          };
        }
        case 'save_files':
          return a.paths;
        case 'enhance_prompt': {
          await new Promise((r) => setTimeout(r, 1200));
          const p = a.params;
          const bg = p.context.remove_background ? '，纯白色背景（#FFFFFF），无投影' : '';
          return { prompt: `${p.prompt}，精致的扁平插画风格，主体居中，柔和光线，色彩干净，细节清晰${bg}，高清`, model: p.model || 'deepseek-v4.1-flash', total_tokens: 500 };
        }
        case 'upload_reference': {
          await new Promise((r) => setTimeout(r, 800));
          if (String(a.path).includes('fail')) throw '上传参考图失败: HTTP 413 图片过大';
          return { path: a.path, url: 'https://example.com/mock.png', expires_at: Math.floor(Date.now() / 1000) + 86400, bytes: 1000, cached: false, compressed: false };
        }
        case 'detect_regions': {
          const b = blobs.get(a.path)!;
          const gap = a.options.gap ?? Math.max(4, Math.round(Math.max(b.width, b.height) * 0.015));
          return { regions: mockDetect(b.canvas, gap), gap, min_size: 12, mode: 'alpha', background: null, labels_found: 0 };
        }
        case 'export_regions':
          return a.items.map((it: { path: string; regions: unknown[] | null }) => ({
            Ok: { input: it.path, out_dir: it.path.replace(/\.\w+$/, '_icons'), size: a.square.size, mode: it.regions ? 'manual' : 'alpha', files: (it.regions ?? [1, 2, 3]).map((_, i) => ({ path: `icon_${i + 1}.png`, name: `icon_${i + 1}`, bytes: 1000, written: true })) },
          }));
        case 'generate_svg_icons':
        case 'render_svg_set': {
          const p = a.params;
          if (cmd === 'generate_svg_icons') await new Promise((r) => setTimeout(r, 1500));
          const shapes = ['<path d="M3 10l9-7 9 7v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1z"/>', '<circle cx="12" cy="12" r="9"/><path d="m15 9-2 4-4 2 2-4z"/>', '<circle cx="9" cy="20" r="1"/><circle cx="18" cy="20" r="1"/><path d="M2 3h3l3 12h11l2-8H6"/>', '<circle cx="12" cy="8" r="4"/><path d="M4 21a8 8 0 0 1 16 0"/>'];
          const list: { label: string; name: string; svg: string }[] =
            cmd === 'render_svg_set'
              ? p.icons
              : p.icons.map((label: string, i: number) => ({
                  label: label.split(/[:：]/)[0],
                  name: /^[\w -]+$/.test(label) ? label : `icon-${String(i + 1).padStart(2, '0')}`,
                  svg: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="${p.style.stroke_width}" stroke-linecap="round" stroke-linejoin="round">${shapes[i % shapes.length]}</svg>`,
                }));
          const icons = list.map((it) => {
            const colored = it.svg.replace('<svg ', `<svg color="${p.color}" `);
            const url = URL.createObjectURL(new Blob([colored], { type: 'image/svg+xml' }));
            return {
              label: it.label,
              name: it.name,
              svg: colored,
              files: [{ path: url, format: 'svg', bytes: colored.length, written: true }, ...p.sizes.map((s: number) => ({ path: url, format: p.format, size: s, bytes: 200 + s * 4, written: true }))],
            };
          });
          return { model: cmd === 'generate_svg_icons' ? p.model || 'deepseek-v4.1-flash' : null, total_tokens: 1400, elapsed_ms: 1500, out_dir: '/mock/svg-icons', icons, missing: [] };
        }
        case 'plugin:dialog|open':
          return null;
        default:
          return null;
      }
    },
    { shouldMockEvents: true },
  );
  // 需在 mockIPC 之后调用，否则窗口元数据会被覆盖
  mockWindows('main');
}
