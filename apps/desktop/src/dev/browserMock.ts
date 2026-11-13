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
