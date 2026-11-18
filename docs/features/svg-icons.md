---
title: SVG 图标生成与渲染
slug: svg-icons
status: active
entry_points:
  - "桌面端：图片生成 → SVG 图标（SvgIconPanel / SvgIconResults）"
  - "MCP：generate_svg_icons、render_svg"
created: 2026-11-17
last_updated: 2026-11-17
---

# SVG 图标生成与渲染

## 概述

简单的线性 / 面性图标不走 AI 绘图，而是让对话模型直接写 SVG，再在本地渲染成多尺寸位图。比图像模型快（8 个图标约 6 秒，图像模型 60-155 秒），体积小（128px PNG 约 0.7-1.3KB，AI 绘图切出的同尺寸图标约 10KB），颜色和线宽精确，生成后能直接换色。写实、渐变、插画类图片仍用 AI 图片生成（`generate_image`，文档待补）。

## 使用方式

- 桌面端：「图片生成」页顶部切到「SVG 图标」。填写图标清单（每行一个，或用逗号 / 顿号分隔；`名称：说明` 可补充画法），选风格和导出尺寸后点「生成 N 个图标」。结果按"套"显示，每个图标可复制 SVG 代码或单独保存；整套可「全部保存」到目录、「加入图片列表」继续压缩 / 转格式，或用颜色选择器换色（只重新渲染，不再调用模型）。
- MCP：
  - `generate_svg_icons`：一次生成整套，返回每个图标的文件、清洗后的 SVG 和一张白底总览预览图。
  - `render_svg`：渲染 Agent 自己写的 SVG（`svg` 参数）或项目里的 `.svg`（`path` 参数），返回文件和 256px 白底预览。Agent 能自己写 SVG 时优先用它，不消耗接口额度。

## 参数与默认值

`generate_svg_icons`（桌面端同名字段一致）：

| 参数 | 含义 | 默认值 | 取值范围 / 说明 |
|---|---|---|---|
| icons | 图标清单 | 必填 | 1-12 个，名称 ≤40 字；`名称：说明` 或 `名称:说明` |
| variant | 线性 / 面性 | outline | outline / filled |
| stroke_width | 线宽（24×24 画布） | 2 | 0.5-4，仅线性 |
| corner | 端点与转角 | round | round / square |
| style_notes | 额外风格要求 | 无 | 自由文本 |
| color | 图标颜色 | #1F2937 | #rgb / #rrggbb / #rrggbbaa / CSS 颜色名 |
| sizes | 位图边长 | [64, 128, 256] | 8-4096，最多 12 个，自动去重 |
| format | 位图格式 | png | MCP：png / webp / avif / jpeg；桌面端：png / webp |
| out_dir | 输出目录 | .imgtool/icons | 项目内（仅 MCP） |
| model | 对话模型 | 「设置」里的提示词模型，未设置时 deepseek-v4.1-flash | 环境变量 IMGTOOL_PROMPT_MODEL 可覆盖（MCP） |
| on_conflict | 重名处理 | rename | rename / overwrite / error |

`render_svg`：`svg` 与 `path` 二选一；`name` 默认取文件名或 `icon`；`sizes`、`format`、`color` 同上；`quality` 位图质量（PNG 不传时无损）；`save_svg` 传源码时默认 true、传路径时默认 false；`out_dir` 传源码时默认 `.imgtool/icons`、传路径时默认与该文件同目录；`on_conflict` 默认 error；支持 `dry_run`。

## 输出

- 每个图标：`<name>.svg`（清洗后，根元素带 `color`）+ `<name>-<尺寸>.<格式>`，尺寸为正方形，图形等比居中、四周透明。
- 文件名：名称是英文 / 数字时直接用（转成小写短横线），中文名用模型给的英文短名（如 `shopping-cart`），都没有时用 `icon-01`。同一套里重名追加 `-序号`。
- 桌面端先写到应用缓存目录 `svg-icons/set-<时间戳>-<序号>/`，点「保存」才复制到用户选的目录（重名自动加序号）。换色会写到新的缓存目录。

## 行为与边界

- 一次请求生成整套，保证线宽、圆角、视觉大小一致。模型漏掉的图标会列为失败，其余照常导出；全部失败时返回错误。
- 模型被要求只用 `currentColor`，所以 `color` 能统一换色；SVG 没有用 currentColor 时会给出提示，颜色不生效。
- 保存和渲染前统一按白名单清洗：去掉 `<script>`、事件属性、`<foreignObject>`、`<image>`、`<style>`、注释、外部 `href` 和 `url(http…)`；只保留图形、渐变、裁剪 / 遮罩和文字元素。被去掉的内容会在结果的 warnings 里列出（`data-*` 属性静默丢弃）。DTD 会被拒绝。
- 渲染不读取外部文件或网络资源，只允许内嵌 `data:` 图片。
- DeepSeek 系列模型会自动带上 `thinking: disabled`；不关闭时约一半请求会把 token 全用在推理上、没有输出。
- 模型对冷门或抽象概念可能画得不准确（实测 deepseek-v4.1-flash 把"设置"画得像太阳，gpt-6.1-sol 画成齿轮但耗时约 4 倍）。不满意时可以在「设置」里换对话模型，或给该项加说明（如 `设置：齿轮`）。
- 单次最多 12 个图标；MCP 是同步调用，8 个图标约 6-30 秒，取决于模型。

## 实现位置

- `crates/imgcore/src/svg.rs`：解析、渲染（resvg）、白名单清洗、总览预览图
- `crates/imgcore/src/project/svg_export.rs`：导出 `.svg` 和多尺寸位图、整套导出、颜色校验
- `crates/imggen/src/svg_icons.rs`：系统提示词、请求、解析模型输出、文件名
- `crates/imggen/src/chat.rs`：对话接口（与提示词优化共用）
- `crates/imgtool/src/mcp/svg_tools.rs`：MCP 工具
- `apps/desktop/src-tauri/src/svg_icons.rs`：桌面端命令 `generate_svg_icons`、`render_svg_set`
- `apps/desktop/src/panels/SvgIconPanel.tsx`、`apps/desktop/src/components/SvgIconResults.tsx`、`apps/desktop/src/generate/svgIcons.ts`

## 验证方式

- `cargo test -p imgcore svg`、`cargo test -p imggen svg_icons`、`cargo test -p imgtool-desktop svg_icons`
- `node scripts/mcp-smoke.mjs`：用假接口覆盖 render_svg / generate_svg_icons 的输出、清洗、命名、缺失图标和参数校验
- 真实接口与模型对比：`IMGGEN_SVG_MODELS=deepseek-v4.1-flash,gpt-6.1-sol HFSYAPI_API_KEY=... cargo test -p imggen live_svg_icons -- --ignored --nocapture`，SVG 写到 `target/live-svg/<模型>/`
- 桌面端界面：`pnpm vite --port 1430` 打开浏览器 Mock，「图片生成 → SVG 图标」生成、换色

## 相关经验

- [DeepSeek 默认思考会耗尽 max_tokens，生成 SVG 时返回空内容](../solutions/integration-issues/deepseek-thinking-consumes-output-tokens.md)

## 变更记录

| 日期 | 变更 |
|---|---|
| 2026-11-17 | 新增 |
