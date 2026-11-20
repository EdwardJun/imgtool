---
title: SVG 输入
slug: svg-input
status: active
entry_points:
  - "桌面端：图片压缩 / 裁剪尺寸 / 格式转换 / 背景处理（添加或拖入 .svg）"
  - "MCP：compress_images、convert_images、crop_image、remove_background、image_info、view_image、import_image、scan_images"
  - "命令行：imgtool compress / convert"
created: 2026-11-17
last_updated: 2026-11-18
---

# SVG 输入

## 概述

`.svg` 可以像其他图片一样被处理：先在本地渲染成位图，再走压缩、裁剪、转格式、去背景流程。用于把矢量图标、logo 导出成 PNG / WebP 等位图。需要多尺寸图标、换色或保留 `.svg` 时用 [SVG 图标生成与渲染](svg-icons.md) 的 `render_svg`。

## 使用方式

- 桌面端：文件选择框和拖放都接受 `.svg`，之后操作与位图相同。
- MCP / 命令行：路径直接传 `.svg`，目录展开和 `scan_images` 会包含 `.svg`。`import_image` 可以把对话里的 SVG（base64 / 地址 / 本机文件）存进项目，原样保存为 `.svg`。

## 参数与默认值

| 项目 | 行为 |
|---|---|
| 渲染尺寸 | 根元素同时声明了 width 和 height 时按声明尺寸；否则长边 512px（按 viewBox 比例） |
| 放大 | 设置了缩放宽 / 高且大于默认渲染尺寸时，直接按目标尺寸渲染（矢量放大不失真） |
| 上限 | 长边最多 8192px（声明更大时等比缩小），文件最大 5MB |
| 输出格式 | "保持原格式"时输出 PNG；也可选 JPEG / WebP / AVIF |
| 背景 | 透明；输出 JPEG 时按设置的背景色填充 |

`image_info` 对 SVG 报告 `format: "svg"`、`decodable: true`，宽高为上表的默认渲染尺寸。

## 输出

与对应功能一致；格式变化时扩展名随之改变，例如 `logo.svg` 压缩后写为同目录 `logo.png`（格式改变时不加 `.min` 后缀）。

## 行为与边界

- 渲染不读取外部文件或网络资源（`<image href="/etc/...">`、相对路径都会被忽略），只渲染内嵌的 `data:` 图片，不执行脚本。
- 只在 SVG 含文字时加载系统字体；文字使用的字体本机没有时会回退到其他字体。
- 无法解析的 SVG 报"解码失败: SVG 解析失败"；`import_image` 会拒绝不完整的 SVG。
- `.svgz`（gzip）可以渲染，但读取尺寸时可能拿不到宽高。
- 仍不支持 HEIC。

## 实现位置

- `crates/imgcore/src/svg.rs`（`parse`、`rasterize`、`dimensions`）
- `crates/imgcore/src/decode.rs`（`sniff_format`、`decode_with_hint`、`probe`）
- `crates/imgcore/src/pipeline.rs`（按缩放目标传渲染尺寸）
- `crates/imgtool/src/intake.rs`（`import_image` 接受 SVG）
- `apps/desktop/src/api.ts`（`IMAGE_EXTENSIONS`）

## 验证方式

- `cargo test -p imgcore`：`svg_input_renders_to_png_and_reports_info`、`svg::tests`
- `node scripts/mcp-smoke.mjs`：SVG 的 scan / image_info / 压缩 / 转 webp / view_image / import_image

## 变更记录

| 日期 | 变更 |
|---|---|
| 2026-11-17 | 新增：SVG 可作为压缩、裁剪、转换、去背景的输入 |
| 2026-11-18 | 桌面端"去除背景"页改名"背景处理"，入口描述同步 |
