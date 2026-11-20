---
title: AI 图片生成
slug: image-generation
status: active
entry_points:
  - "桌面端：图片生成页 → AI 绘图（GeneratePanel、GenerateResults）"
  - "MCP：generate_image、get_generation"
created: 2026-11-19
last_updated: 2026-11-19
---

# AI 图片生成

## 概述

调用 hfsyapi 兼容的图片接口（`POST /v1/image/create` + `GET /v1/image/query/{id}`）按提示词出图，可带参考图，生成后可在本地去背景、调整到精确尺寸、转格式，也可以直接解析成设计稿 spec（见 [设计解析](design-spec.md)）。

## 使用方式

- 桌面端：图片生成页 → AI 绘图。填写提示词（可「优化提示词」交给对话模型改写，可撤销），选模型、比例、质量档位、数量，可添加参考图、开启透明背景（仅支持的模型）、生成后去除背景、生成后解析设计稿、调整到最终尺寸，点「生成图片」。每张图一个任务并发执行，结果卡片可加入图片列表、保存到目录、打开「设计解析」。接口地址与 API Key 在「设置」里填写。
- MCP：`generate_image` 提交任务后立即返回 `task_id`，再调用 `get_generation`（最多等 `wait_seconds` 秒）查询；完成后返回文件路径、尺寸和缩略图（开启 `analyze_design` 时附编号标注图）。接口配置读取环境变量 `IMGTOOL_API_KEY` / `IMGTOOL_API_BASE_URL`，没有时读桌面端「设置」保存的配置文件。

## 参数与默认值

| 参数（MCP） | 含义 | 默认值 | 取值范围 / 说明 |
|---|---|---|---|
| `prompt` | 提示词 | 必填 | 最多 5000 字 |
| `model` | 图像模型 | gpt-image-2 | 只支持 1K；2K/4K 用 flare、nano-banana 等 |
| `ratio` | 画面比例 | 按 `final_size` 自动选，否则 1:1 | 1:1、5:4、4:3、3:2、16:9、21:9、3:1、4:5、3:4、2:3、9:16 |
| `quality` | 质量档位 | 1K | 1K / 2K / 4K（需模型支持） |
| `size` | 直接指定尺寸 | — | 如 1024x1024，优先于 ratio / quality |
| `reference_images` | 参考图 | [] | 项目内路径（自动上传图床）或 http(s) 地址，最多 16 张 |
| `count` | 张数 | 1 | 1-4 |
| `final_size` / `fit` | 本地调整到的最终尺寸 | — / cover | cover、contain、fill |
| `remove_background` | 生成后本地去除纯色背景 | false | |
| `analyze_design` | 生成后解析成设计稿 spec | false | 同 `analyze_design`，结果在 `get_generation` 的 `files[].design` |
| `format` | 保存格式 | png | jpeg / png / webp / avif |
| `out_dir` / `name` | 输出目录 / 文件名前缀 | `.imgtool/generated` / gen | |

桌面端同名选项对应上表；数量 1-4，参考图上限按模型（4-16 张），表单设置记在 localStorage `imgtool.generate.form`。

## 输出

- MCP：`<out_dir>/<name>-<任务后缀>.<ext>`，多张时 `<name>-<任务后缀>-<序号>.<ext>`，重名自动加序号。开启 `analyze_design` 时另写 spec、标注图和图标素材（路径规则见设计解析文档）。
- 桌面端：先保存在应用缓存目录 `generated/gen-<任务>-<序号>.<ext>`，「保存」复制到选择的目录（重名加序号）；开启「生成后解析设计稿」时 spec 写在该图片所在目录的 `.imgtool/specs/` 下。

## 行为与边界

- 单张约 30 秒到 3 分钟，每次生成消耗接口额度。hfsyapi 只接受图片地址作为参考图，本地图片会先压缩（长边 ≤ 2048）并上传到图床，链接 24 小时有效；桌面端首次上传前会提示。
- 去背景、调整尺寸、设计稿解析失败只记为警告，不影响生成结果；全部失败时任务为 failed，部分成功为 partial。
- MCP 任务记录保存在进程内（最多 50 个），MCP 重启后查不到旧任务，但已生成的文件仍在输出目录。

## 实现位置

- `crates/imggen/`：`task_api.rs`（创建 / 轮询 / 下载）、`upload.rs`（参考图上传）、`sizes.rs`（尺寸表）、`prompt.rs`（提示词优化）
- `crates/imgcore/src/postprocess.rs`：去背景、最终尺寸、转格式
- `crates/imgtool/src/gen_tasks.rs`：MCP 后台任务与 `PostProcess`；`crates/imgtool/src/mcp.rs`：`generate_image`、`get_generation`；`crates/imgtool/src/genconfig.rs`：接口配置
- `apps/desktop/src-tauri/src/generate.rs`、`references.rs`；`apps/desktop/src/panels/GeneratePanel.tsx`、`components/GenerateResults.tsx`、`generate/useGenerator.ts`、`generate/models.ts`

## 验证方式

- `cargo test -p imggen`、`cargo test -p imgtool-desktop`
- `node scripts/mcp-smoke.mjs`：假接口覆盖提交、轮询、参考图上传、最终尺寸、去背景、`analyze_design`
- `scripts/mcp-agent-demo.mjs <图片>`：真实接口端到端（消耗一次生成）

## 变更记录

| 日期 | 变更 |
|---|---|
| 2026-11-19 | 补建文档；新增 `analyze_design` 参数与桌面端「生成后解析设计稿」开关、结果卡片「设计解析」按钮 |
