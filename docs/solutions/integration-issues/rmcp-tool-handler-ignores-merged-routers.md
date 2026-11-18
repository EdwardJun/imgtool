---
title: "rmcp 的 #[tool_handler] 默认只用 Self::tool_router()，合并的路由不生效"
date: 2026-11-17
category: integration-issues
module: imgtool MCP Server
problem_type: integration_issue
component: tooling
symptoms:
  - "新的 #[tool_router(router = ...)] impl 块里的工具不出现在 tools/list 中"
  - "结构体字段 tool_router 被标成 dead_code（#[allow(dead_code)]）"
root_cause: wrong_api
resolution_type: code_fix
severity: medium
tags: [rmcp, mcp, tool-router, tool-handler, macro]
---

# rmcp 的 #[tool_handler] 默认只用 Self::tool_router()，合并的路由不生效

## Problem

把 SVG 工具拆到单独的 `#[tool_router(router = svg_tool_router)]` impl 块，并在构造时写 `tool_router: Self::tool_router() + Self::svg_tool_router()`，新工具仍然不出现在 `tools/list`。

## Symptoms

- 冒烟测试列出的工具只有原来的 14 个。
- `ImgToolServer.tool_router` 字段原本就带着 `#[allow(dead_code)]`，说明它从来没被读过。

## What Didn't Work

- 以为把合并后的路由存进字段就会被使用。rmcp-macros 3.9.0 的 `#[tool_handler]` 生成的 `call_tool`、`list_tools`、`get_tool` 默认都调用 `Self::tool_router()`（每次新建一份只含主 impl 块工具的路由），不读字段。

## Solution

在 handler 上显式指定路由表达式：

```rust
// crates/imgtool/src/mcp.rs
#[tool_handler(router = self.tool_router)]
impl ServerHandler for ImgToolServer { /* ... */ }
```

并去掉字段上的 `#[allow(dead_code)]`。之后 `node scripts/mcp-smoke.mjs` 列出 16 个工具。

## Why This Works

宏把 `router` 参数原样展开到三个生成的方法里。传 `self.tool_router` 后，三处都使用构造时合并好的路由。

## Prevention

- 增加 `#[tool_router(router = ...)]` 块时，同时检查 `#[tool_handler]` 是否带 `router = self.tool_router`。
- 冒烟测试断言工具总数（`scripts/mcp-smoke.mjs` 的"暴露 N 个工具"），新增工具时同步更新数量。
- 排查前先确认二进制是新的，见 [tauri dev 会用旧的 sidecar 覆盖 target/debug/imgtool](../developer-experience/tauri-dev-overwrites-imgtool-binary.md)；这次两个问题同时出现，症状很像。

## Related Issues

- rmcp-macros 3.9.0 `src/tool_handler.rs`（`ToolHandlerAttribute::default` 的 `router`）
