---
title: 背景处理（去除背景 / 更换底色 / 证件照）
slug: remove-background
status: active
entry_points:
  - "桌面端：背景处理页 RemoveBgPanel（透明背景 / 更换底色）"
  - "MCP：remove_background"
  - "命令行：imgtool compress / convert --remove-bg [--bg-fill] [--photo-size] [--bg-method ai]"
  - "桌面端：图片生成 → 生成后去除背景（复用透明背景参数）"
created: 2026-11-18
last_updated: 2026-11-19
---

# 背景处理（去除背景 / 更换底色 / 证件照）

## 概述

本地按颜色抠掉纯色或近纯色背景（白底产品图、图标、AI 生成的纯色背景图、白墙或蓝布前拍的人像），然后二选一：

- 透明背景：输出带透明通道的 PNG / WebP / AVIF。
- 更换底色：把主体合成到新的纯色上，输出不透明的 JPG / PNG。可以再按人像自动构图，输出一寸、二寸等精确像素的证件照，并限制文件大小。

抠图有两种方式：

- 按颜色（默认，透明背景模式）：纯本地算法，原图背景越接近纯色效果越好；背景杂乱或发丝细碎时边缘会粗糙，结果里会给出警告。
- AI 人像（更换底色模式默认）：本地运行 MODNet 人像抠图模型，只适合人物照片。背景不必是纯色，发丝边缘更自然，白衬衫等与背景同色的衣物不会被误删。首次使用需下载约 25MB 模型，之后离线可用，图片不会上传。

## 使用方式

- 桌面端：顶部"背景处理"页，面板顶部用 Segmented 切换"透明背景 / 更换底色"。
  - "抠图方式"选"AI 人像"或"按颜色"，两种模式各自记住（透明背景默认按颜色，更换底色默认 AI 人像）。选 AI 且模型未下载时显示"下载模型"按钮和进度，下载完自动重新预览。
  - 两种模式共用原背景识别参数（自动识别或指定颜色、容差、边缘柔和、去除白边）；按颜色时还有"只去除与边缘相连的背景"，更换底色按颜色时还有"人像模式"。
  - 更换底色、按颜色时默认开启"人像模式"：被人物挡住的同色区域（西装里的白衬衫、领口）会保留。拍的不是人像时可以关掉。
  - 更换底色：选新底色（白 / 蓝 / 红色块或取色器），选证件照尺寸（原尺寸 / 一寸 / 二寸 / 小一寸 / 小二寸 / 自定义）。选了尺寸后默认按人像自动构图；点"调整构图"在左侧拖动比例锁定的框微调当前图片，"恢复自动构图"丢弃微调。批量导出时没微调过的图片各自自动构图。
  - 尺寸换了以后，之前为别的尺寸微调的框不再使用。
- MCP：`remove_background`。透明背景只传 `paths`；换底色加 `fill_color`；证件照再加 `photo_size`，需要时加 `max_kb`；人像照片建议加 `method: "ai"`（首次调用自动下载模型）。
- 命令行：`imgtool compress photo.jpg --remove-bg --bg-method ai --bg-fill blue --photo-size 1inch --max-kb 50 -f jpg`（首次使用在终端提示并下载模型）。

## 参数与默认值

| 参数（桌面端 / MCP / CLI） | 含义 | 默认值 | 取值范围 / 说明 |
|---|---|---|---|
| 抠图方式 / `method` / `--bg-method` | `color` 按颜色 / `ai` MODNet 人像抠图 | 桌面端透明背景 color、更换底色 ai；MCP / CLI color | ai 时忽略"只去除与边缘相连""人像模式"，容差和柔和只用于清理边缘 |
| 背景颜色 / `color` / `--bg-color` | 要去除的原背景色 | 自动识别四边最常见的颜色 | `#rrggbb`；边缘全透明时取紧贴透明区的一圈像素 |
| 容差 / `tolerance` / `--bg-tolerance` | 与背景色距离在此范围内的像素完全去除 | 12 | 0-100（桌面端滑块 0-60） |
| 边缘柔和 / `softness` / `--bg-softness` | 容差之外这段距离做半透明过渡 | 8 | 0-100（桌面端滑块 0-40） |
| 只去除与边缘相连的背景 / `contiguous` / `--bg-global` 取反 | 从四边洪水填充，主体内部同色区保留 | 开 | 人像模式开启时不使用（由可见性规则代替） |
| 人像模式 / `portrait` / `--bg-portrait true\|false` | 背景必须能从左、右或上边缘沿直线"看到" | 桌面端换底色时开；MCP / CLI 在设置了底色或证件照尺寸时开，否则关 | 规则见下方"人像模式" |
| 去除白边 | 半透明边缘反混合掉原背景色 | 开 | MCP / CLI 不暴露，始终开 |
| 裁掉透明空白 / `trim`、`trim_padding` / `--trim [N]` | 裁到主体外接矩形加边距 | 关、0px | 仅透明背景；设置了证件照尺寸时忽略 |
| 新底色 / `fill_color` / `--bg-fill` | 更换底色 | 桌面端蓝 `#438edb`；MCP / CLI 不传则透明 | `white` `#ffffff`、`blue` `#438edb`、`red` `#ff0000`（也认 白 / 蓝 / 红），或任意 `#rrggbb` |
| 证件照尺寸 / `photo_size` / `--photo-size` | 自动构图后缩放到精确像素 | 原尺寸（不裁剪） | `1inch` 295×413、`2inch` 413×579、`small-1inch` 260×378、`small-2inch` 413×531，或 `宽x高`（1-10000）；也认中文名 |
| 限制文件大小 / `max_kb` / `--max-kb` | 文件大小上限 | 桌面端关、开启时 50KB | 只降低质量，证件照不缩小分辨率 |
| 输出格式 | | 透明 PNG；换底色桌面端 JPG、MCP 默认 JPEG、CLI 保持原格式 | 透明背景只能 PNG / WebP / AVIF；换底色可 JPG / PNG（MCP 也接受 webp / avif） |

编码：透明 PNG 无损，WebP / AVIF 质量 90（MCP 为无损 / 质量 100）；换底色 JPG 质量 92、PNG 无损；限制大小时走目标体积搜索。

### AI 人像抠图（MODNet）

- 模型：MODNet 官方人像抠图权重 `modnet_photographic_portrait_matting.onnx`（25,888,640 字节，Apache-2.0），用纯 Rust 的 tract 在 CPU 上推理，无系统依赖。
- 存放位置（桌面端与 MCP / CLI 共用，下载一次即可）：macOS `~/Library/Application Support/com.imgtool.desktop/models/`，Windows `%APPDATA%\com.imgtool.desktop\models\`，Linux `~/.config/com.imgtool.desktop/models/`。
- 下载：依次尝试自有服务器 `http://106.75.18.68/imgtool/models/modnet_photographic_portrait_matting.onnx`（国内访问快，约 1MB/s）、Hugging Face、GitHub（HivisionIDPhotos 发布页）、hf-mirror.com，校验大小和 SHA-256（`07c308cf…84df9`），先写 `.part` 再改名。自有服务器用 HTTP，靠哈希校验保证完整性，文件被改过会被拒绝并换下一个源。全部失败时报错并提示手动下载。
- 离线 / 自定义：环境变量 `IMGTOOL_MATTING_MODEL` 指向本地模型文件，优先于默认位置；指向的文件不存在时直接报错，不会去下载。
- 推理：长边缩到 512、宽高取 32 的倍数，输出遮罩两端各截掉 3% 噪声后放大回原尺寸。实测 2048×2800 照片约 0.5 秒（Apple Silicon），按尺寸缓存推理计划。
- 边缘清理：模型在低分辨率上推理，放大后边缘有过渡带，会把原背景色带进新底色形成光晕。所以模型判断人像透明度 < 50% 的像素，再按容差 / 柔和看颜色是否接近原背景，接近的就去掉；模型确定是人像的区域不看颜色。
- AI 时不报"背景不是纯色"；没识别到人像时警告"AI 抠图没有识别到人像（只适合人物照片）"。模型未下载时报"AI 人像抠图模型还没有下载"。
- 安装包体积：imgtool 发行版二进制从约 9MB 增加到约 32MB（tract 推理库）；模型不打进安装包。

### 人像模式

1. 全图按颜色算出每个像素的"背景程度"（容差、边缘柔和照常生效）。
2. 从每一行的左右两端、每一列的顶端向内扫描：颜色像背景的像素记为可见；遇到不像背景的像素时，连续不超过约 0.8% 短边（至少 3px）的可以越过（发丝、图片边缘的细线），更宽的（脸、西装）就停下。
3. 只有可见的像素被当作背景。所以发丝之间的小块背景能去掉，被西装、肩膀挡住的白衬衫、领口保留。
4. 边缘匹配率只统计上边和左右两边的上半段，并向内缩 2-8px，避开衣服和图片边框。

局限：衣物在某一行上能直接连到背景（没穿外套、侧身露出白衬衫边缘）时，那一段仍会被当成背景；发丝边缘只按颜色过渡，浅色头发或灰墙会发糙。

### 自动构图规则

基于去背景后的透明度遮罩，不做人脸检测（`crates/imgcore/src/portrait.rs`）：

1. 主体范围：不透明度 ≥ 64 的像素外接矩形。
2. 头部：从主体顶部向下逐行看主体宽度（只认连续不透明的片段，忽略边缘细线和噪点）。行宽回落到头部最宽处 85% 以下（脖子）后不再计入头宽；之后行宽超过头宽 1.5 倍的行视为肩膀。头部像素的平均横坐标为水平中心。
3. 识别到头和肩：框宽 = 头宽 / 0.6；否则框高 = 主体高 / 0.9（框住整个主体）。
4. 按目标宽高比取框，头顶留白 10%，水平以头部为中心；超出原图时等比缩小并贴边。
5. 没识别到主体（如全部被去除）时退化为居中裁剪，并给出警告。

## 输出

- 输出路径与命名沿用通用规则（`plan_output_path`）：格式不变时同目录 `name.min.ext`，格式变化时 `name.<新扩展名>`（例如 `a.png` 换底色输出 JPG 得到 `a.jpg`）。桌面端已存在时自动加序号；MCP 默认按项目预设的 `on_conflict`。
- 处理报告（桌面端预览、MCP 结果每张的 `background` / `auto_crop`）：识别出的背景色、边缘匹配率、去除比例、主体范围（含头宽、肩膀位置估计），以及自动构图实际使用的框（原图坐标）。

## 行为与边界

- 边缘匹配率：普通模式忽略匹配最差的一条边（产品常贴着底边），人像模式只看上边和两侧上半段；低于 90% 时警告"背景不是纯色"。几乎没去掉像素时警告"没有找到可去除的背景"。
- 透明背景下 MCP 传 `format: jpeg` 报错，提示改用 `fill_color`；桌面端透明模式不提供 JPG。
- 限制大小时质量降到下限（默认 40）仍超标，返回最小结果并警告"质量已降到 N（保持 宽x高 不缩小），仍无法达到目标体积"。
- 证件照小于原图时会缩小，原图比目标尺寸小时会放大（可能变糊）。
- 自定义尺寸未填完整时桌面端按原尺寸处理。
- 只处理 GIF 第一帧；SVG 先渲染成位图（见 [SVG 输入](svg-input.md)）。
- 图片生成页"生成后去除背景"复用透明背景参数，不使用底色和证件照设置；点"调整参数"会切到背景处理页的透明背景模式。

## 实现位置

- `crates/imgcore/src/background.rs`：`BackgroundRemoval`（含 `fill`、`portrait`）、`remove_background`、`keep_only_visible`、`portrait_border_match`、`subject_bounds`、`fill_background`、`RemovalStats`
- `crates/imgcore/src/portrait.rs`：证件照尺寸、底色预设、`frame` 自动构图
- `crates/imgcore/src/matting.rs`：MODNet 推理（`matte`、`set_model_path`、`IMGTOOL_MATTING_MODEL`）；`background.rs` 的 `RemovalMethod` 与 AI 边缘清理
- `crates/imggen/src/models.rs`：模型位置、下载源、SHA-256 校验；`crates/imgtool/src/matting.rs`（MCP / CLI 自动下载）；`apps/desktop/src-tauri/src/matting.rs`（`matting_model_status`、`download_matting_model`、`model-download-progress` 事件）
- `crates/imgcore/src/pipeline.rs`：`CropSpec::Subject` 换算、换底色时允许 JPEG、`TargetSize.keep_dimensions`
- `crates/imgcore/src/types.rs`：`CropSpec::Subject`、`PixelRect`、`ProcessReport.auto_crop`
- `crates/imgtool/src/mcp.rs`（`remove_background`、`RemoveBgParams`）、`crates/imgtool/src/options.rs`（`parse_fill`、`id_photo_resize`、`Overrides.resize` / `keep_dimensions`）、`crates/imgtool/src/main.rs`（CLI 参数）
- `apps/desktop/src/panels/RemoveBgPanel.tsx`、`apps/desktop/src/options.ts`（`PHOTO_SIZES`、`FILL_PRESETS`、`buildOptions`）、`apps/desktop/src/App.tsx`（构图编辑）

## 验证方式

- `cargo test -p imgcore`：`background::tests`（含 `portrait_mode_keeps_enclosed_white_shirt`）、`portrait::tests`，以及集成测试 `replace_background_keeps_jpeg_and_fills_color`、`id_photo_auto_frames_subject_and_meets_size_limit`、`target_size_keep_dimensions_does_not_shrink`
- `cargo test -p imgtool`：`id_photo_overrides_beat_preset_resize`、`parses_crop_args`
- `cargo test -p imggen models`：下载的大小 / 哈希校验
- 真实模型推理（默认忽略）：`IMGTOOL_MATTING_MODEL=<模型路径> cargo test -p imgcore matting -- --ignored`；MCP 冒烟测试设置同一个环境变量时会检查 `method: "ai"`，否则跳过
- `node scripts/mcp-smoke.mjs`：透明输出、拒绝透明 JPEG、换蓝底一寸 JPG ≤ 30KB、拒绝无效底色
- 真实照片：白墙前穿西装白衬衫的 2048×2800 半身照，CLI 导出一寸蓝底 / 二寸红底，确认衬衫领口完整、无"背景不是纯色"警告，并放大检查发丝边缘
- 桌面端：`pnpm vite --port 1430` 浏览器 Mock 下切换"更换底色"、选尺寸、调整构图（Mock 不做真实抠图和主体识别）

## 相关经验

- [透明图缩放出现白边](../solutions/logic-errors/transparent-resize-color-bleed.md)
- [人像让"主体在中间"的假设失效（白衬衫被去除、误报非纯色、头部过小）](../solutions/logic-errors/portrait-subject-touches-edge-heuristics.md)
- [低分辨率抠图遮罩放大后出现光晕](../solutions/logic-errors/upscaled-matting-mask-halo.md)

## 变更记录

| 日期 | 变更 |
|---|---|
| 2026-11-18 | 补建文档；新增更换底色、证件照尺寸与自动构图、文件大小上限；桌面端页签"去除背景"改名"背景处理"；边缘匹配率改为忽略最差一条边 |
| 2026-11-18 | 新增人像模式（换底色默认开）：保护被主体挡住的白衬衫等同色区域，可去除发丝间的背景；人像模式下边缘匹配率只看上边和两侧上部；头肩识别改为到脖子为止计头宽、忽略边缘细线 |
| 2026-11-18 | 新增 AI 人像抠图（MODNet，本地推理，首次使用下载模型）：桌面端"抠图方式"、MCP `method`、CLI `--bg-method`；更换底色默认 AI；AI 边缘按颜色二次清理消除光晕 |
| 2026-11-19 | 模型首选下载源改为自有服务器（106.75.18.68），原有下载源保留为备用 |
