# Agent Instructions

imgtool：图片压缩 / 裁剪 / 格式转换 / 去背景 / 智能裁剪 / AI 图片生成的桌面端（Tauri + React，`apps/desktop`）与 MCP Server（`crates/imgtool`），核心能力在 `crates/imgcore`、`crates/imggen`。

## 文档留档规则

本仓库有两类文档，职责不同，不要互相复制内容：

| 目录 | 记录什么 | 何时写 |
|---|---|---|
| `docs/features/` | 功能**现在**是什么、怎么用、参数与默认值、边界行为、实现位置 | 每次新增或修改功能 |
| `docs/solutions/` | 踩坑经验：失败的做法、被推翻的假设、真实缺陷的根因与预防 | 工作推翻了一个错误认知时 |

### 什么算"功能"

用户能直接用到的能力：桌面端面板或交互、MCP 工具、命令行参数、项目预设（`.imgtool.toml`）的配置项、对外的输出路径与文件命名规则。纯内部重构、测试、构建脚本不算。

### 新增功能：必须留档

1. 按 `docs/features/_template.md` 新建 `docs/features/<feature-slug>.md`（slug 用小写英文短横线，如 `smart-crop`）。
2. 在 `docs/features/README.md` 的索引表加一行。
3. 文档与功能代码放在**同一个 commit / PR** 里，不能拆成"之后补文档"。

### 修改功能：必须同步更新文档

只要改动影响下列任一项，就要在同一个 commit / PR 里更新对应功能文档：

- 用户可见的行为、界面流程、提示文案
- 参数、默认值、取值范围、MCP 工具的入参 / 出参 / 描述
- 输出路径、文件命名、覆盖策略
- 错误处理与边界情况
- 实现位置（文件移动或重命名后，文档里的路径要跟着改）

更新时：改正文使其描述**改动后的现状**（不保留过时说法），更新 frontmatter 的 `last_updated`，并在"变更记录"末尾追加一行。

- 功能还没有文档：先按模板补建，再写本次改动。
- 功能被删除：把文档 `status` 改为 `removed` 并在变更记录说明原因，索引表同步标注；不要直接删文件。
- 纯重构且行为不变：不必改正文，但要确认"实现位置"里的路径仍然准确。

### 完成报告

结束一项工作时，在回复里列出本次新建 / 更新的文档路径；判断为不需要改文档时，写明理由（例如"纯重构，行为不变"）。不要静默跳过。

### 经验沉淀（ce-compound）

After a solved, verified problem, automatically invoke the `ce-compound` skill with `mode:headless` at the completion checkpoint when the work corrected a wrong belief about how something behaves: an approach that failed, a review finding that exposed a real defect, or an assumption, document, or first diagnosis that turned out wrong. A naming, style, or preference change does not count. Work that went as first planned does not need it. Do not judge whether the lesson deserves a document: `ce-compound` decides whether it qualifies and what holds it (a test, a lint rule, a coding standard, or a learning), and writes nothing when nothing qualifies. Capture at the checkpoint so what it writes can ship in the PR that produced it. This repository treats captured learnings as tracked, committed knowledge: `docs/solutions/` is tracked by git.

功能文档里需要引用相关经验时，链接到 `docs/solutions/` 下的文件，不要把经验正文抄进功能文档。

`docs/solutions/` 按问题类型分子目录（如 `logic-errors/`），每篇带 YAML frontmatter（`module`、`tags`、`problem_type`），在已有经验覆盖的模块里实现或排查问题时可以先检索。
