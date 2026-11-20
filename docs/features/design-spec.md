---
title: 设计解析（设计稿 → 结构化描述）
slug: design-spec
status: active
entry_points:
  - "桌面端：设计解析页（DesignPanel + DesignSpecOverlay）；图片生成页「生成后解析设计稿」开关、结果卡片「设计解析」按钮"
  - "MCP：analyze_design、annotate_design、generate_design_icons；generate_image 的 analyze_design 参数"
  - "项目预设：.imgtool.toml 的 [spec] 段"
created: 2026-11-19
last_updated: 2026-11-20
---

# 设计解析（设计稿 → 结构化描述）

## 概述

把 UI 设计稿（整页）或单个好看的样式（卡片、按钮、标签、输入框）转换成 Agent 能直接读的结构化描述：嵌套节点树、每个元素的像素坐标（绝对坐标 + 相对父节点）、类型、填充 / 线性渐变 / 描边 / 圆角 / 外阴影、文字颜色与估算字号字重、推断出的自动布局（横排 / 竖排、间距、内边距、对齐）。设计稿里的图标会同时裁成透明底素材，需要时可交给 AI 一次重绘成高清版。

几何与样式由本地纯算法得出（色块分层 + 递归 XY-cut，不调用大模型、不花额度、不做 OCR）；语义（这是什么、写了什么字）由 Agent 看编号标注图后写回，或在桌面端手动标注。

## 使用方式

- MCP（典型顺序）：
  1. `analyze_design { path }`：返回摘要、精简文本树、JSON（spec 路径、调色板、图标素材列表）和编号标注图。
  2. Agent 看标注图，调用 `annotate_design { spec, annotations: [{ id, role?, name?, text?, description?, type? }], notes? }` 按编号写入语义。图标的 `name` 会作为素材文件名。
  3. 需要高清图标时调用 `generate_design_icons { spec, ids?, style? }`，再用 `get_generation` 查询结果。
  4. 用 `generate_image` 生成 UI 稿时传 `analyze_design: true`，`get_generation` 完成后直接返回 spec 路径和标注图。
- 桌面端：顶部「设计解析」页 → 选图 → 「解析设计稿」。左侧预览可放大缩小：右上角缩放条（缩小 / 百分比 / 放大 / 适应窗口，百分比是相对原图像素的实际比例，点击百分比在"适应窗口"和"原图 100%"之间切换），按住 ⌘ 或 Ctrl 滚动、触控板双指捏合以光标为中心缩放，⌘ + / ⌘ - / ⌘ 0 快捷键（焦点在输入框时不响应）；放大后可以滚动查看，也可以拖动画面：缩放条最左边的「抓手」打开后直接拖，或者随时按住空格 / 鼠标中键拖（拖动画面时不会新增框）；范围为适应窗口的 25%-1600%，切换图片时回到适应窗口。图上按类型着色显示节点框和编号：点击选中、拖动移动、拖四角缩放、在空白处拖动新增框、Delete 删除（子节点上移到父节点）。框移动 / 缩放 / 新增后会自动重新取这块区域的样式。右侧有层级树、节点属性（类型、角色、名称、文字、说明可编辑；样式与布局只读）、「整理层级」「保存修改」、图标素材列表（每个图标显示原图尺寸，小于 24px 的标「小」并默认不参与重绘，可勾选「也重绘过小的图标」；还没命名的图标会提示先命名）与「AI 重绘高清图标」，底部「打开解析文件夹」按钮（或点击下方的文件夹路径）在 Finder / 资源管理器里打开这张图的解析文件夹。图片生成页可打开「生成后解析设计稿」，结果卡片上的「设计解析」按钮会把图加入列表并切到设计解析页。

## 参数与默认值

`analyze_design`：

| 参数 | 含义 | 默认值 | 取值范围 / 说明 |
|---|---|---|---|
| `path` | 设计稿或样式图片 | 必填 | 项目内路径；对话里的图先用 `import_image` |
| `mode` | page 整页 / component 单个样式 | 自动 | 画面主体是单个色块（且不占满画布）时判为 component，根节点收缩到该色块 |
| `tolerance` | 与底色的容差 | 自动 | 0-100；自动时按背景噪点估算（约 2，RGB 距离 8-30） |
| `max_depth` | 最大嵌套层数 | 6 | 1-12 |
| `keep_annotations` | 重新解析时保留旧标注 | true | 按位置 IoU ≥ 0.8 匹配旧节点 |
| `export_icons` | 自动裁出图标素材 | true | |
| `detail` | outline 精简树 / full 附完整 spec JSON | outline | |

`annotate_design`：`spec` 可传 `.spec.json` 或原图路径；`annotations[].id` 可写 `n12` 或 `12`；字段传空字符串表示清空；`type` 可改为 frame / rect / text / icon / image / line。

`generate_design_icons`：`ids` 默认全部 icon 节点中原图长边 ≥ 24px 的（更小的跳过并在返回的 `skipped` 里说明原因；显式传 `ids` 时按传入的全部重绘），一次最多 16 个；`style` 风格描述；`model` 默认 gpt-image-2；`quality` 默认 1K；`size` 素材边长，默认取 `[spec] icon_size`。

`generate_image.analyze_design`：默认 false。

项目预设 `.imgtool.toml`：

```toml
[spec]
# 图标素材目录（项目内相对路径），不填则放在 .imgtool/specs/<相对目录>/<原名>/icons/
# icons_dir = "src/assets/icons"
icon_size = 256   # 16-2048
```

桌面端设置（localStorage `imgtool.design`）：类型、容差、自动裁出图标、显示层数、显示分组、重绘风格、重绘模型。

## 输出

- 项目根目录：MCP 为图片所在的项目根；桌面端为最近的 `.imgtool.toml` 所在目录，没有预设时为图片所在目录。
- 一张图的全部产物放在同一个解析文件夹 `<根>/.imgtool/specs/<图片相对目录>/<原名>/` 里：`<原名>.spec.json`（spec）、`<原名>.spec.png`（编号标注图，长边 ≤ 1600）、`icons/`（图标素材）、`<原名>.icons-ref.png`（AI 重绘参考图：裁出的原始图标网格）、`<原名>.icons-context.png`（AI 重绘上下文参考图：完整设计稿上用红框和编号框出图标）、`<原名>-icons-sheet*.png`（AI 生成的图标网格原图）。重新解析直接覆盖。MCP `analyze_design` 的 JSON 返回 `dir`，桌面端结果带 `dir`。
- 图标素材：默认 `<解析文件夹>/icons/`，`[spec] icons_dir` 配置后直接写到该目录。文件名为标注的 `name`（清洗后，如 `home.png`），没有名称时为 `icon-n7.png`；同一 spec 里重名追加编号（`home-n9.png`），与不属于本 spec 的已有文件重名时自动加序号（`keep-1.png`），不覆盖别的文件。透明底正方形 PNG，默认 256px、四周留白 10%。
- 标注名称后素材自动改名；重新解析时，`.imgtool/specs` 下不再使用的旧裁剪素材会被清理（自定义 `icons_dir` 里的文件不清理）；AI 高清版（`asset.source = generated`）不会被裁剪图覆盖。
- AI 重绘：参考图与生成的网格图都写在解析文件夹里（MCP：`<原名>-icons-sheet-<编号>.png`；桌面端：`<原名>-icons-sheet.png`，每次覆盖）；拆出的图标覆盖原素材文件，spec 里改为 `generated`。
- spec 里的所有路径相对项目根目录，统一用 `/`。

spec 结构（version 1）要点：

- 顶层：`version`、`generator`、`source{path,width,height}`、`kind`（page / component）、`background`、`transparent`、`palette[{color,ratio}]`（最多 8 个）、`annotated`、`notes`、`root`。
- 节点：`id`（深度优先编号 n1、n2…，与标注图数字一致）、`type`、`role` / `name` / `text` / `description`（语义）、`bounds{x,y,width,height}`（原图像素）、`relative{x,y}`（相对父节点）、`style{fill,stroke,corner_radius,shadow}`、`text_style{color,font_size,line_height,lines,weight,align,confidence}`、`layout{mode,gap,gap_uniform,padding[上右下左],align,cross_align}`、`asset{path,source,width,height}`、`confidence`、`source`（auto / agent / manual）、`children`。空字段省略。
- `fill` 为 `{type:"solid",color}` 或 `{type:"linear",angle,stops:[{pos,color}]}`（角度与 CSS linear-gradient 一致）；`corner_radius` 为一个数或 `[左上,右上,右下,左下]`；`shadow` 为 `{x,y,blur,color(#rrggbbaa),confidence}`。
- 没有样式的 `frame` 是自动分组（标注图里画灰色虚线，outline 里写作 `group`），用来表达行 / 列结构。单个子节点的容器按 row 描述。

## 行为与边界

- 图标与文字的区分：长边小于画面短边 1.2%（至少 12px）的小块是色块（如列表圆点），不当图标；粗体字里被当成实心色块的单字（如「2026」的数字）会一个带一个地并回文字行，整行都是实心字时 ≥3 个同行同色的也按文字处理；按列投影能切成几段占满整块高度的块（并排的字，如徽标里的「01」）按文字处理；明显偏宽的小色块（宽高比 > 1.25）不会被整体折叠成图标。
- 识别能力：带底色或描边的色块（卡片、按钮、输入框、色条、头像底）及其内部递归；内容底色与页面接近的卡片（只露出一圈卡片底色）；白底上只靠阴影显出的白卡片；文字行（相邻同行同色的字合并，竖排里字号接近、对齐一致的行合并为段落）；图标（含圆环里的小点、购物车轮子等部件，App 图标这类不含文字的小方块整体当图标）；照片 / 插画（颜色多的块，或色块里全是低对比碎块）；分割线。
- 估算值：字号 = 墨迹高度 × 1.15、字重由笔画宽度推算、阴影只在纯色底上估算（偏移、模糊、透明度），都带 `confidence`。白底白卡片的圆角由阴影包住的角估算；只有阴影时可能没有圆角值。
- 渐变：只拟合线性渐变（两端色标），残差过大的视为图片；AI 生成的不严格线性的渐变按残差放宽，内部低对比的拟合残留会被丢弃。
- 不做 OCR：`text` 为空，需要 Agent 或人工填写。图标内部被识别成"文字"、大号单字被识别成色块等情况，可通过 `annotate_design` 或桌面端改 `type`。
- 长边超过 2560 的图先缩小再分析，坐标换算回原图。透明底按白底合成分析，`transparent=true`，根节点不写填充。
- 图标裁剪：按图标外围一圈的众数色去底并还原边缘颜色；实心徽标（不透明部分占 45% 以上）内部被包住的白色图案保留，线性图标内部的底色变透明。
- AI 重绘：选中的图标按 ⌈√n⌉ 列网格排在一张图上生成，消耗一次生成额度。传两张参考图：原图标拼成的网格，以及框出图标位置的完整设计稿（长边 ≤ 1536），让模型结合位置和旁边的文字理解含义；提示词里写每个图标的名称 / 角色 / 说明、离它最近的已标注文字（如「旁边的文字是『申报时间』」）和主色。默认跳过原图长边 < 24px 的图标（放大后没有可辨认的造型），全部都太小时直接报错、不花额度；MCP 返回 `skipped`、`unnamed`（没有名称 / 说明的）、`low_res`（强制加入的过小图标）、`context_reference`，素材列表里的 `native`（原图长边像素）和 `low_res` 标出较模糊的裁剪素材。拆分时先用图标识别，数量正好时按识别结果，否则按网格格子取；某格为空时保留原素材并在警告里说明。最多 16 个，更多请用 `ids` 分批。
- `annotate_design` 有未知编号时整体不修改并报错；未解析过的图会提示先调用 `analyze_design`。`generate_image` 的自动解析失败只记为警告，不影响生成结果。
- 桌面端「保存修改」会按当前的框和名称重新裁出图标素材并重画标注图；「AI 重绘」前会先保存未保存的修改。「整理层级」按包含关系重新嵌套、重新分组并推断布局，已有编号不变，新分组接着最大编号排。
- 性能：1K 竖屏稿（941×1672）release 构建约 20-50ms。

## 实现位置

- `crates/imgspec/`：`model.rs`（spec 结构）、`canvas.rs`（画布、纯色 / 线性渐变拟合）、`segment.rs`（色块分层、描边 / 阴影 / 空心色块、递归）、`pieces.rs`（笔画合并与 text / icon / image / line 分类）、`group.rs`（XY-cut 分组、段落合并、导航条目对齐）、`style.rs`（圆角、阴影、字号字重）、`layout.rs`（布局推断、编号、整理层级）、`overlay.rs`（编号标注图）、`store.rs`（路径、读写、语义写回、保留旧标注）、`icons.rs`（图标裁剪、改名、AI 网格重绘计划与拆分）、`outline.rs`（精简文本树）、`fixtures.rs`（合成测试图）；开发示例 `examples/analyze.rs`（`cargo run -p imgspec --example analyze -- <图> [--out 目录] [--json] [--crop x,y,w,h]`）
- `crates/imgcore/src/project/preset.rs`、`preset_template.toml`：`[spec]` 段（`SpecPolicy`）
- `crates/imgtool/src/mcp/design_tools.rs`：三个 MCP 工具；`crates/imgtool/src/gen_tasks.rs`：`PostProcess`（生成后解析 / 拆分图标网格）；`crates/imgtool/src/mcp.rs`：`generate_image.analyze_design`、`get_generation` 返回设计稿结果与标注图预览、INSTRUCTIONS
- `apps/desktop/src-tauri/src/design_spec.rs`：Tauri 命令 `analyze_design`、`load_design_spec`、`save_design_spec`、`reanalyze_region`、`normalize_spec`、`generate_design_icons`、`open_folder`（用系统 open / explorer / xdg-open 打开目录，不经过 shell）；`generate.rs`：`analyze_design` 生成参数
- `apps/desktop/src/design/`（类型、树操作、`useDesign`）、`components/DesignSpecOverlay.tsx`、`components/FitBox.tsx`（缩放与以光标为中心的滚轮缩放）、`components/ZoomBar.tsx`、`panels/DesignPanel.tsx`、`App.tsx`（设计解析页）、`panels/GeneratePanel.tsx`、`components/GenerateResults.tsx`

## 验证方式

- `cargo test -p imgspec`：合成图精度（层级、边界 ±2px、圆角 ±2px、描边 ±1px、渐变色 ΔE<5、阴影偏移 ±2px / 模糊 ±30%、字号 ±15%、段落行高、布局间距与对齐、大图缩放）、真实稿暴露的问题复现（胶囊渐变按钮、与页面同底色的卡片内容、噪点串行）、文件流程（落盘、写回、改名、保留标注、AI 网格拆分、自定义素材目录、整理层级、局部重新取样式）。
- `cargo test -p imgcore preset`：`[spec]` 段解析与校验；`cargo test -p imgtool-desktop design_spec`：根目录判断与前端参数反序列化。
- `node scripts/mcp-smoke.mjs`：analyze → annotate（改名）→ 重新解析保留标注 → generate_design_icons（假接口）→ generate_image(analyze_design)。
- 人工验收：用 `generate_image` 生成电商首页、登录页（9:16，1K）后自动解析，核对标注图；对登录页图标标注名称后真实调用一次 `generate_design_icons`，7 个图标按 3×3 网格生成并正确拆分。
- 桌面端：`pnpm vite --port 1430`（浏览器 Mock）检查设计解析页的解析、选中、改名、新增框、保存流程。

## 相关经验

- [设计稿解析在合成图上全部通过，换成 AI 生成的 UI 稿就认错](../solutions/logic-errors/ai-ui-mockup-segmentation-pitfalls.md)
- [AI 重绘出的设计稿图标含义不对：根因是误识别和原图过小](../solutions/logic-errors/blurry-design-icons-are-misclassified-not-low-res.md)

## 变更记录

| 日期 | 变更 |
|---|---|
| 2026-11-19 | 新增：imgspec 设计解析、MCP 三个工具、图标素材裁剪与 AI 网格重绘、桌面端设计解析页、生成后自动解析、`[spec]` 预设 |
| 2026-11-20 | 一张图的解析产物（spec、标注图、图标、参考图、网格图）统一放进 `.imgtool/specs/<相对目录>/<原名>/`；桌面端新增「打开解析文件夹」；修复桌面端 `.imgtool` 目录下的图标缩略图无法显示（资源协议不匹配点开头的目录）、AI 重绘后缩略图不刷新 |
| 2026-11-20 | 桌面端设计解析页预览支持放大缩小（缩放条、⌘/Ctrl + 滚轮、捏合、快捷键） |
| 2026-11-20 | 修正把列表圆点、粗体数字、「01」徽标误认成图标；AI 重绘默认跳过原图 < 24px 的图标并报告未命名图标，增加框出图标位置的上下文参考图和旁边文字提示；桌面端预览支持拖动（抓手 / 空格 / 中键），图标列表显示原图尺寸与「小」标记 |
