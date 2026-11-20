---
title: 低分辨率抠图遮罩放大后，换底色出现一圈原背景色光晕
date: 2026-11-18
category: logic-errors
module: imgcore AI 人像抠图
problem_type: logic_error
component: tooling
symptoms:
  - "MODNet 遮罩本身看起来干净，但换成红底后头发和肩膀外围有一圈发白的光晕"
  - "放大看发丝外侧有几像素宽、半透明的原背景色（白墙）"
root_cause: logic_error
resolution_type: code_fix
severity: medium
tags: [matting, modnet, alpha-mask, upscale, halo, remove-background, id-photo]
---

# 低分辨率抠图遮罩放大后，换底色出现一圈原背景色光晕

## Problem

MODNet 在长边 512 左右的分辨率上推理，2048×2800 的照片要把遮罩放大约 4 倍。放大后的边缘过渡带有十几个原图像素宽，这些半透明像素里混着原背景色。合成到新底色上，就形成一圈浅色光晕。

## Symptoms

- 只看遮罩（黑白图）完全正常，看不出问题。
- 换红底后，头发外沿和肩膀轮廓外有明显的白色光圈；换白底时看不出来，所以容易漏掉。

## What Didn't Work

- 直接用模型遮罩作为 alpha，只靠"去除白边"（按识别出的背景色反混合）消除光晕。反混合能修正颜色，但过渡带太宽、alpha 不准：本该是背景的像素还保留着 20%-40% 的不透明度，于是背景色被"还原"成了一圈半透明的浅色。

## Solution

模型拿不准的像素再按颜色判断一次（`background.rs` 的 `RemovalMethod::Ai` 分支）：

```rust
let ai = 1.0 - mask / 255.0;          // 模型给出的背景程度
if ai <= 0.5 { return ai; }            // 模型确定是人像：不看颜色（白衬衫、脸不受影响）
let by_color = /* 按容差 / 柔和计算的颜色背景程度 */;
ai.max(by_color)                       // 边缘上颜色像原背景的，按颜色去掉
```

## Why This Works

模型擅长回答"哪里是人"，颜色擅长回答"这个像素是不是原背景色"。只在模型认为偏背景的过渡带里用颜色补充判断，就能把放大带来的背景色残留去掉。白衬衫这类与背景同色、但模型确定是人像的区域，因为不进入颜色判断，所以不会被误删（颜色抠图正是在这一点上失败的）。

## Prevention

- 检查抠图效果时，换一个与原背景反差大的底色（白墙照片换红底 / 蓝底），并放大发丝和肩膀轮廓看。只看遮罩或只换相近底色看不出光晕。
- 遮罩来自低分辨率推理时，默认假设边缘过渡带偏宽，合成前要有边缘清理步骤。

## Related Issues

- [人像证件照让"主体在中间"的假设失效](portrait-subject-touches-edge-heuristics.md)：颜色抠图在人像上的局限，也是引入 AI 抠图的原因。
- 功能说明：`docs/features/remove-background.md`
