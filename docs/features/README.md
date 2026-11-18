# 功能文档索引

每个用户可见的功能对应一篇文档，描述它现在的行为。新增功能和修改功能时的更新规则见仓库根目录 `AGENTS.md` 的"文档留档规则"。新文档从 `_template.md` 复制。

状态说明：`active` 已有文档；`待补` 功能已存在但还没写文档（下次修改该功能时先补建）；`removed` 功能已删除。

| 功能 | 文档 | 入口 | 状态 |
|---|---|---|---|
| 图片压缩 | `compress.md` | 桌面端 CompressPanel；MCP `compress_images`、`scan_images` | 待补 |
| 裁剪 | `crop.md` | 桌面端 CropPanel；MCP `crop_image` | 待补 |
| 格式转换 | `convert.md` | 桌面端 ConvertPanel；MCP `convert_images` | 待补 |
| 去背景 | `remove-background.md` | 桌面端 RemoveBgPanel；MCP `remove_background` | 待补 |
| 智能裁剪 / 拆分图标 | `smart-crop.md` | 桌面端 SmartCropPanel；MCP `detect_icons`、`split_icons` | 待补 |
| AI 图片生成 | `image-generation.md` | 桌面端 GeneratePanel（图片生成 → AI 绘图）；MCP `generate_image`、`get_generation` | 待补 |
| SVG 图标生成与渲染 | `svg-icons.md` | 桌面端 SvgIconPanel（图片生成 → SVG 图标）；MCP `generate_svg_icons`、`render_svg` | active |
| SVG 输入 | `svg-input.md` | 桌面端各处理页添加 .svg；MCP 压缩 / 转换 / 裁剪 / 去背景 / `image_info` / `view_image` / `import_image` | active |
| MCP 项目上下文与图片导入 | `mcp-project-context.md` | 桌面端 McpModal；MCP `get_project_context`、`init_project_preset`、`import_image`、`image_info`、`view_image` | 待补 |
