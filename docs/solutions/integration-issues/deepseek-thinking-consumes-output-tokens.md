---
title: DeepSeek 默认思考会耗尽 max_tokens，生成 SVG 时返回空内容
date: 2026-11-17
category: integration-issues
module: imggen 对话接口（SVG 图标生成）
problem_type: integration_issue
component: tooling
symptoms:
  - "deepseek-v4.1-flash 生成 8 个 SVG 图标，约一半请求耗时 35-45 秒后 content 为空"
  - "finish_reason 为 length，usage.completion_tokens_details.reasoning_tokens 等于 max_tokens"
root_cause: config_error
resolution_type: code_fix
severity: high
tags: [deepseek, reasoning, thinking, chat-completions, hfsyapi, svg-icons]
---

# DeepSeek 默认思考会耗尽 max_tokens，生成 SVG 时返回空内容

## Problem

通过 hfsyapi 调用 deepseek-v4.1-flash 生成一整套 SVG 图标，约 50% 的请求没有任何 SVG 输出，报"模型没有返回 SVG"。同样的请求偶尔 7 秒就成功。

## Symptoms

- 失败的请求都在 35-45 秒左右返回，`content` 为空字符串，`reasoning_content` 很长。
- `finish_reason: "length"`，推理 token 用满 `max_tokens`（6100）。
- 简短的系统提示词、4 个图标时多数能成功，规则越多、图标越多越容易失败。

## What Didn't Work

- 最初把它当成模型偶发不稳定，错误提示只写"模型没有返回 SVG，可换模型重试"，看不出原因。
- `reasoning_effort: "low"`、`enable_thinking: false` 在 hfsyapi 上都无效，各 2 次全部失败。

## Solution

1. `ChatReply` 增加 `finish_reason` 和 `reasoning_tokens`，截断时提示"输出达到长度上限被截断（推理 N token）"（`crates/imggen/src/chat.rs`、`svg_icons.rs`）。
2. 对 DeepSeek 模型在请求体里加 `"thinking": {"type": "disabled"}`：

```rust
// crates/imggen/src/svg_icons.rs
let chat = if chat.model().to_ascii_lowercase().contains("deepseek") {
    chat.with_extra(serde_json::json!({ "thinking": { "type": "disabled" } }))
} else { chat };
```

关闭后 6/6 成功，每次 5.7-6.7 秒、约 1400 token。gpt-6.1-sol 收到该参数会忽略，不报错。

## Why This Works

DeepSeek 默认开启思考，推理 token 和正文共用 `max_tokens`。写一套图标是"长规则 + 长输出"的任务，模型容易在推理阶段反复斟酌坐标，把额度用完后正文为空。关闭思考后直接输出 SVG。

## Prevention

- 调用对话模型时，失败信息里带上 `finish_reason` 和推理 token，空输出先看是否被截断。
- 换用其他思考模型时，先用 `IMGGEN_CHAT_EXTRA='{"thinking":{"type":"disabled"}}' IMGGEN_SVG_MODELS=<模型> cargo test -p imggen live_svg_icons -- --ignored --nocapture` 连续跑几次，确认成功率和耗时。
- 提示词优化（`PromptEnhancer`）目前输出短，没有出现这个问题；若改成长输出任务，也应考虑关闭思考。

## Related Issues

- [SVG 图标生成与渲染](../../features/svg-icons.md)
