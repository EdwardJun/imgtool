---
title: MCP 项目上下文与图片导入
slug: mcp-project-context
status: active
entry_points:
  - "桌面端：Agent 接入（McpModal）"
  - "MCP：get_project_context、init_project_preset、import_image、image_info、view_image"
created: 2026-11-19
last_updated: 2026-11-19
---

# MCP 项目上下文与图片导入

## 概述

让 Agent 在项目上下文里使用 imgtool：知道项目根目录和团队预设 `.imgtool.toml`，把对话里的图片存进项目，查看图片内容与基础信息。桌面端「Agent 接入」可一键把 imgtool 写进常见 Agent 客户端的 MCP 配置。

## 使用方式

- 桌面端：顶部「Agent 接入」查看 MCP 命令与配置片段，为 Claude Code、Cursor、Kiro 等客户端一键写入配置。
- MCP：处理图片前先调用 `get_project_context`；没有预设时可用 `init_project_preset` 创建；对话里的图片用 `import_image` 存进项目后，再用返回的路径调用其他工具；`view_image` 看缩略图，`image_info` 读格式、宽高、体积。

## 参数与默认值

| 工具 / 参数 | 含义 | 默认值 | 说明 |
|---|---|---|---|
| `init_project_preset.overwrite` | 已存在时覆盖 | false | |
| `import_image.url` / `base64` / `source_path` | 三选一的图片来源 | — | `source_path` 需是本机绝对路径，可在项目外；base64 可带 data URL 前缀 |
| `import_image.filename` / `dir` | 文件名 / 保存目录 | 按来源推断 / `.imgtool/inbox` | 扩展名按实际格式修正，重名自动加序号，最大 30MB |
| `view_image.max_size` | 缩略图最长边 | 512 | 64-1024 |
| `image_info.paths` | 图片路径列表 | 必填 | |

预设 `.imgtool.toml`（`init_project_preset` 生成的模板带注释）：

| 段 | 字段 | 默认值 |
|---|---|---|
| `[defaults]` | `format`、`quality`（1-100）、`max_kb`、`max_width`、`max_height` | 质量 80，其余不设 |
| `[output]` | `mode`（suffix / overwrite / dir）、`suffix`、`dir`、`on_conflict`（error / overwrite / rename） | suffix、`.min`、`.imgtool/out`、error |
| `[scan]` | `exclude`、`warn_kb`、`warn_dimension` | []、500、2560 |
| `[spec]` | `icons_dir`（项目内相对路径）、`icon_size`（16-2048） | 不设、256（见 [设计解析](design-spec.md)） |

未知字段、越界值、`icons_dir` 含 `..` 或绝对路径时预设解析失败并报出文件路径。

## 输出

- `get_project_context`：根目录、预设文件路径与内容、支持的输入 / 输出格式、生成接口是否已配置（不返回 Key）、输出命名规则。
- `init_project_preset`：在项目根目录写 `.imgtool.toml`。
- `import_image`：返回项目内相对路径与图片信息。

## 行为与边界

- 项目根目录优先级：`--root` > 客户端 Roots > 环境变量 `IMGTOOL_ROOT` > 当前工作目录；所有读写限制在根目录内（`import_image.source_path` 是唯一例外，会把文件复制进项目）。
- 预设从图片所在目录逐级向上查找，不越过所在根目录。

## 实现位置

- `crates/imgtool/src/mcp.rs`、`workspace.rs`、`intake.rs`
- `crates/imgcore/src/project/preset.rs`、`preset_template.toml`
- `apps/desktop/src-tauri/src/mcp_config.rs`、`commands.rs`；`apps/desktop/src/components/McpModal.tsx`

## 验证方式

- `cargo test -p imgcore preset`、`cargo test -p imgtool`
- `node scripts/mcp-smoke.mjs`：根目录来自 Roots、创建预设、导入（base64 / url / 项目外文件）、拒绝项目外路径

## 变更记录

| 日期 | 变更 |
|---|---|
| 2026-11-19 | 补建文档；预设新增 `[spec]` 段（`icons_dir`、`icon_size`） |
