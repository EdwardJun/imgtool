---
title: 透明图缩放后边缘出现白边（未预乘 alpha 插值）
date: 2026-11-17
category: logic-errors
module: imgcore 缩放
problem_type: logic_error
component: tooling
symptoms:
  - "去背景后的图标缩小后，外轮廓一圈发白的半透明像素，在深色背景上很明显"
  - "原图边缘没有亮色像素，缩放后半透明边缘的 RGB 接近 (250,250,254)"
root_cause: wrong_api
resolution_type: code_fix
severity: medium
tags: [alpha, premultiplied-alpha, resize, lanczos, halo, remove-background, app-icon]
---

# 透明图缩放后边缘出现白边（未预乘 alpha 插值）

## Problem

带透明通道的图片经过 imgcore 缩放（压缩时限制尺寸、裁剪后缩放、生成图 final_size、智能裁剪导出）后，外轮廓出现一圈发白的半透明边。用 imgtool 生成应用图标、缩到 1024 时发现。

## Symptoms

- 去背景后的 1254px 图标，原图边缘没有偏白的半透明像素；缩到 1024 后有约 800 个。
- 透明像素的 RGB 保留了被去掉的背景色（白色，alpha=0），缩放后这些颜色出现在边缘。

## What Didn't Work

- 第一版预乘实现把预乘值按 `×255 / alpha16` 反算，量纲差了 257 倍，结果所有半透明和不透明像素都变成接近黑色（文件从 600KB 掉到 14KB）。当时的单元测试只断言"边缘不发白"（R < 90），黑色同样满足，测试照样通过。是对真实图片测边缘颜色时才发现。

## Solution

新增 `crates/imgcore/src/resize.rs::resize_exact`：图片有非不透明像素时，先转成 16 位预乘 RGBA 插值，再反预乘回 8 位；全不透明时直接走 `image` 的 `resize_exact`。`pipeline.rs`（`apply_resize`、`scale_by`、目标体积循环）和 `detect.rs::render_square` 的缩放都改用它。

```rust
// 预乘：颜色16 × alpha8 / 255
let p = |c: u8| ((c as u32 * 257 * a16 + 127) / 255) as u16;
// 反预乘：预乘值 × 65535 / alpha16，再降到 8 位
let u = |c: u16| ((((c as u32 * 65535 + a32 / 2) / a32).min(65535) + 128) / 257) as u8;
```

## Why This Works

`image` 的 `resize_exact` 对 RGBA 四个通道各自独立插值。alpha=0 的像素虽然看不见，它的 RGB 仍按卷积权重混进相邻像素。预乘后透明像素的颜色分量都是 0，对邻居的颜色贡献也是 0，边缘只混入真实可见的颜色。

## Prevention

- 对缩放、混合这类变换写测试时，断言**结果颜色等于期望值**（容差内），不要只断言"不是某种坏颜色"。`resize.rs` 的测试同时检查：边缘保持原色、不透明区域原样、均匀半透明图缩放后颜色和 alpha 不变，并保留一条对照断言证明朴素缩放确实会发白。
- 新增缩放调用时用 `imgcore::resize::resize_exact`，不要直接调 `DynamicImage::resize_exact` / `imageops::resize`。
- Lanczos 在硬边缘本身有几个色阶的振铃，颜色断言需留少量容差（当前 ±8）。

## Related Issues

- 去背景会把背景像素设为 alpha=0 但保留原 RGB，这是透明区"藏色"的来源：`crates/imgcore/src/background.rs`。
