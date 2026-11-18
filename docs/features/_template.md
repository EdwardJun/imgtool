---
title: 功能名称
slug: feature-slug
status: active          # active | removed
entry_points:           # 用户从哪里用到这个功能
  - "桌面端：XxxPanel"
  - "MCP：tool_name"
created: YYYY-MM-DD
last_updated: YYYY-MM-DD
---

# 功能名称

## 概述

一两句话：这个功能解决什么问题，给谁用。

## 使用方式

- 桌面端：入口位置与操作步骤。
- MCP：工具名、典型调用顺序。

## 参数与默认值

| 参数 | 含义 | 默认值 | 取值范围 / 说明 |
|---|---|---|---|
| | | | |

## 输出

输出路径、文件命名、已存在时的覆盖策略。

## 行为与边界

不支持的输入、错误提示、限制（尺寸、格式、并发等）。

## 实现位置

- `crates/...`
- `apps/desktop/src/...`

## 验证方式

覆盖这个功能的测试或手动验证步骤。

## 相关经验

- 链接到 `docs/solutions/` 下的相关文档（没有就删掉本节）。

## 变更记录

| 日期 | 变更 |
|---|---|
| YYYY-MM-DD | 新增 |
