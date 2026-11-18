---
title: tauri dev 会用旧的 sidecar 覆盖 target/debug/imgtool
date: 2026-11-17
category: developer-experience
module: 桌面端开发 / MCP 调试
problem_type: developer_experience
component: development_workflow
severity: medium
applies_when:
  - "pnpm tauri dev 在运行时修改了 crates/imgtool（MCP 工具、CLI）"
  - "用 target/debug/imgtool 跑 mcp-smoke.mjs、真实接口脚本，或 Agent 的 MCP 配置指向它"
symptoms:
  - "刚 cargo build -p imgtool 过，几分钟后 tools/list 又少了新工具，调用报 tool not found"
  - "target/debug/imgtool 的大小和时间戳与 apps/desktop/src-tauri/binaries/imgtool-<triple> 完全相同"
tags: [tauri, sidecar, external-bin, mcp, dev-workflow, stale-binary]
---

# tauri dev 会用旧的 sidecar 覆盖 target/debug/imgtool

## Context

桌面端把 imgtool 作为 sidecar（`bundle.externalBin: binaries/imgtool`）。`pnpm tauri dev` 启动时 `beforeDevCommand` 跑一次 `pnpm sidecar`，把当时编译的 imgtool 复制到 `apps/desktop/src-tauri/binaries/imgtool-<triple>`。之后每次 Rust 代码变化触发桌面端重新编译，Tauri 都会把 `binaries/` 里的 sidecar 复制到桌面端可执行文件旁边，也就是 `target/debug/imgtool`。

所以 tauri dev 开着时，手动 `cargo build -p imgtool` 产出的新二进制，会在下一次桌面端重编译时被启动那一刻的旧 sidecar 覆盖。2026-11-17 新增 SVG 工具时，冒烟测试先通过、过一会儿又失败（16 个工具变回 14 个），真实接口脚本报 `tool not found`。最初以为是 MCP 路由注册有问题，查二进制大小和时间戳才发现被换回了旧版。

## Guidance

- 改了 `crates/imgtool` 后，用 `pnpm sidecar`（或 `node scripts/build-sidecar.mjs`）重新生成 sidecar，或者直接重启 `pnpm tauri dev`，不要只 `cargo build -p imgtool`。
- 跑 `node scripts/mcp-smoke.mjs` 或真实接口脚本前，先确认二进制是新的：

```bash
ls -la target/debug/imgtool apps/desktop/src-tauri/binaries/imgtool-*
# 两者时间戳应晚于最近一次修改；冒烟测试第一项会打印工具列表
```

- Agent 的 MCP 配置（如 `~/.kiro/settings/mcp.json`）指向 `target/debug/imgtool` 时同样会受影响，重新生成 sidecar 后还要在 Agent 里重启 MCP。

## Why This Matters

被覆盖时没有任何报错，表现为"新工具时有时无"，很容易误判成代码问题去改路由、改注册逻辑。

## When to Apply

- tauri dev 运行期间修改 MCP 工具或 CLI。
- 冒烟测试结果与预期的工具数量不符时，先查二进制再查代码。

## Examples

修复前：`cargo build -p imgtool` → 冒烟通过 → 修改 imgcore 触发桌面端重编译 → 冒烟报"暴露 16 个工具"失败。

修复后：`pnpm sidecar`（或重启 tauri dev）→ `ls -la` 确认 `target/debug/imgtool` 与 `binaries/` 下文件都是新时间戳 → 冒烟全部通过。

## Related

- `scripts/build-sidecar.mjs`、`apps/desktop/src-tauri/tauri.conf.json`（`externalBin`、`beforeDevCommand`）
