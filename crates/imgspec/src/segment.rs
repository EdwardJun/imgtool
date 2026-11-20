//! 分层切分：在容器（底色已知）内找出前景连通块，判断哪些是带底色/描边的色块（递归进入），
//! 其余作为叶子元素（文字、图标、图片、分割线）交给 pieces 模块合并与分类。

use crate::canvas::{self, Bg, Canvas};
use crate::color::{self, Rgb};
use crate::group;
use crate::model::{Bounds, LayoutMode, Node, NodeType, Stroke};
use crate::pieces::{self, Piece};
use crate::style;

#[derive(Clone, Copy)]
pub(crate) struct Ctx<'a> {
    pub cv: &'a Canvas,
    /// 前景阈值（RGB 距离）
    pub thr: f32,
    pub max_depth: usize,
    pub long: f32,
    pub short: f32,
}

impl Ctx<'_> {
    /// 判定"同一种颜色"的阈值
    fn thr_core(&self) -> f32 {
        self.thr.max(10.0)
    }
    pub fn line_thick(&self) -> u32 {
        ((self.long * 0.004).round() as u32).max(3)
    }
}

/// 圆角矩形区域：容器内部分析时排除圆角外和贴边的像素
#[derive(Clone, Copy)]
pub(crate) struct Shape {
    pub r: Bounds,
    pub radius: f32,
}

impl Shape {
    fn contains(&self, x: u32, y: u32) -> bool {
        let r = &self.r;
        if x < r.x || y < r.y || x >= r.right() || y >= r.bottom() {
            return false;
        }
        let rad = self.radius;
        if rad < 1.0 {
            return true;
        }
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let cx = if fx < r.x as f32 + rad {
            r.x as f32 + rad
        } else if fx > r.right() as f32 - rad {
            r.right() as f32 - rad
        } else {
            return true;
        };
        let cy = if fy < r.y as f32 + rad {
            r.y as f32 + rad
        } else if fy > r.bottom() as f32 - rad {
            r.bottom() as f32 - rad
        } else {
            return true;
        };
        ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt() <= rad - 1.0
    }
}

pub(crate) struct Comp {
    pub b: Bounds,
    /// 画布像素下标
    pub pix: Vec<u32>,
}

/// 前景掩码 + 8 邻域连通标记
fn label(ctx: &Ctx, area: &Bounds, bg: &Bg, shape: Option<&Shape>) -> Vec<Comp> {
    let cv = ctx.cv;
    let (aw, ah) = (area.width as usize, area.height as usize);
    let mut mask = vec![false; aw * ah];
    for y in 0..ah {
        for x in 0..aw {
            let (gx, gy) = (area.x + x as u32, area.y + y as u32);
            if shape.map(|s| !s.contains(gx, gy)).unwrap_or(false) {
                continue;
            }
            mask[y * aw + x] = color::dist(cv.get(gx, gy), bg.at(gx, gy)) > ctx.thr;
        }
    }
    let mut seen = vec![false; aw * ah];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    for start in 0..aw * ah {
        if !mask[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let mut pix = Vec::new();
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % aw, i / aw);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            pix.push(((area.y as usize + y) * cv.w as usize + area.x as usize + x) as u32);
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx < 0 || ny < 0 || nx >= aw as i64 || ny >= ah as i64 {
                        continue;
                    }
                    let j = ny as usize * aw + nx as usize;
                    if mask[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        if pix.len() < 3 {
            continue;
        }
        let b = Bounds::new(area.x + x0 as u32, area.y + y0 as u32, (x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32);
        out.push(Comp { b, pix });
    }
    out
}

fn px_xy(cv: &Canvas, i: u32) -> (u32, u32) {
    (i % cv.w, i / cv.w)
}

/// 连通块在其外接框内的成员位图
struct Members {
    b: Bounds,
    bits: Vec<bool>,
}

impl Members {
    fn new(cv: &Canvas, c: &Comp) -> Self {
        let mut bits = vec![false; c.b.area() as usize];
        for &i in &c.pix {
            let (x, y) = px_xy(cv, i);
            bits[((y - c.b.y) * c.b.width + (x - c.b.x)) as usize] = true;
        }
        Members { b: c.b, bits }
    }
    fn has(&self, x: u32, y: u32) -> bool {
        x >= self.b.x && y >= self.b.y && x < self.b.right() && y < self.b.bottom() && self.bits[((y - self.b.y) * self.b.width + (x - self.b.x)) as usize]
    }
}

/// 从 k 的某条边向外（到 b 的边）逐像素取平均色，只取该边中间 50%
fn side_profile(cv: &Canvas, k: &Bounds, b: &Bounds, side: usize) -> Vec<Rgb> {
    let mut out = Vec::new();
    let (len, start) = if side < 2 { (k.width, k.x) } else { (k.height, k.y) };
    let (a, e) = (start + len / 4, start + (len * 3 / 4).max(len / 4 + 1));
    let depth = match side {
        0 => k.y - b.y,
        1 => b.bottom() - k.bottom(),
        2 => k.x - b.x,
        _ => b.right() - k.right(),
    };
    for d in 1..=depth {
        let mut sum = [0f32; 3];
        let mut n = 0f32;
        for t in a..e {
            let (x, y) = match side {
                0 => (t, k.y - d),
                1 => (t, k.bottom() - 1 + d),
                2 => (k.x - d, t),
                _ => (k.right() - 1 + d, t),
            };
            let p = cv.get(x, y);
            for i in 0..3 {
                sum[i] += p[i];
            }
            n += 1.0;
        }
        out.push(sum.map(|s| s / n.max(1.0)));
    }
    out
}

/// 是否是向外渐淡的阴影：低饱和、比底色暗、亮度单调回升
fn is_halo(prof: &[Rgb], bg: Rgb) -> bool {
    if prof.is_empty() {
        return false;
    }
    let lb = color::lum(bg);
    let mut prev = f32::MAX;
    let mut bad = 0;
    for p in prof {
        let diff = [p[0] - bg[0], p[1] - bg[1], p[2] - bg[2]];
        let spread = diff.iter().fold(f32::MIN, |a, &v| a.max(v)) - diff.iter().fold(f32::MAX, |a, &v| a.min(v));
        let dark = lb - color::lum(*p);
        if spread > 16.0 || dark < -2.0 || dark > 110.0 {
            return false;
        }
        if dark > prev + 2.0 {
            bad += 1;
        }
        prev = dark;
    }
    bad * 5 <= prof.len()
}

/// 剖面开头的描边：几个像素颜色一致且明显不同于底色，之后陡降
fn stroke_on(prof: &[Rgb], bg: Rgb, fill: Rgb, thr: f32) -> Option<(usize, Rgb)> {
    let c1 = *prof.first()?;
    let tol = thr.max(14.0);
    let k = prof.iter().take_while(|p| color::dist(**p, c1) <= tol).count();
    let contrast = color::dist(c1, bg);
    if k > 4 || contrast < 2.0 * thr || color::dist(c1, fill) < 2.0 * thr {
        return None;
    }
    // 抗锯齿 / 缩放产生的边缘：颜色正好落在底色与填充色的连线上
    let d = [fill[0] - bg[0], fill[1] - bg[1], fill[2] - bg[2]];
    let len2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
    if len2 > 1.0 {
        let v = [c1[0] - bg[0], c1[1] - bg[1], c1[2] - bg[2]];
        let t = (v[0] * d[0] + v[1] * d[1] + v[2] * d[2]) / len2;
        let perp = [v[0] - t * d[0], v[1] - t * d[1], v[2] - t * d[2]];
        let pd = (perp[0] * perp[0] + perp[1] * perp[1] + perp[2] * perp[2]).sqrt();
        if (0.05..=0.95).contains(&t) && pd < 12.0 && k <= 2 {
            return None;
        }
    }
    if k < prof.len() && color::dist(prof[k], bg) > 0.6 * contrast {
        return None;
    }
    Some((k, c1))
}
pub(crate) struct BoxHit {
    pub node: Node,
    /// 色块的外框（含描边，不含阴影）
    pub rect: Bounds,
    /// 递归分析内部时使用的区域、底色与形状
    pub inner: Option<(Bounds, Bg, Shape)>,
    /// 内部分析用的前景阈值（渐变填充不是严格线性时要放宽）
    pub thr: Option<f32>,
}

/// 连通块内与 fill 接近的像素：外接框与数量
fn core_of(cv: &Canvas, c: &Comp, fill: &Bg, tol: f32) -> (Option<Bounds>, usize) {
    let (mut x0, mut y0, mut x1, mut y1, mut n) = (u32::MAX, u32::MAX, 0, 0, 0usize);
    for &i in &c.pix {
        let (x, y) = px_xy(cv, i);
        if color::dist(cv.get(x, y), fill.at(x, y)) <= tol {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            n += 1;
        }
    }
    if n == 0 {
        (None, 0)
    } else {
        (Some(Bounds::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1)), n)
    }
}

fn shrink(b: &Bounds, frac: f32) -> Bounds {
    let dx = (b.width as f32 * frac) as u32;
    let dy = (b.height as f32 * frac) as u32;
    Bounds::new(b.x + dx, b.y + dy, (b.width - 2 * dx).max(1), (b.height - 2 * dy).max(1))
}

/// 阴影把相邻的两张卡片连成了一块：按"填充色核心"拆开
fn split_by_cores(ctx: &Ctx, c: &Comp) -> Option<Vec<Comp>> {
    let cv = ctx.cv;
    if c.pix.len() < 400 {
        return None;
    }
    let (mode, _) = color::mode_color(c.pix.iter().map(|&i| cv.get(i % cv.w, i / cv.w)), 4)?;
    let tol = ctx.thr_core();
    let b = c.b;
    let (bw, bh) = (b.width as usize, b.height as usize);
    let mut core = vec![false; bw * bh];
    for &i in &c.pix {
        let (x, y) = px_xy(cv, i);
        if color::dist(cv.get(x, y), mode) <= tol {
            core[((y - b.y) as usize) * bw + (x - b.x) as usize] = true;
        }
    }
    let mut seen = vec![false; bw * bh];
    let mut boxes: Vec<(Bounds, usize)> = Vec::new();
    let mut stack = Vec::new();
    for s in 0..bw * bh {
        if !core[s] || seen[s] {
            continue;
        }
        seen[s] = true;
        stack.push(s);
        let (mut x0, mut y0, mut x1, mut y1, mut n) = (usize::MAX, usize::MAX, 0, 0, 0);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % bw, i / bw);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            n += 1;
            let nb = [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)];
            for (nx, ny) in nb {
                if nx < bw && ny < bh {
                    let j = ny * bw + nx;
                    if core[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        boxes.push((Bounds::new(b.x + x0 as u32, b.y + y0 as u32, (x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32), n));
    }
    let largest = boxes.iter().map(|b| b.1).max()?;
    let mut big: Vec<Bounds> = boxes.iter().filter(|(bb, n)| *n >= (largest / 20).max(64) && bb.width >= 6 && bb.height >= 6).map(|b| b.0).collect();
    // 被大核心包住的小核心（如卡片里同色的块）不算
    let all = big.clone();
    big.retain(|k| !all.iter().any(|o| o != k && o.contains_ratio(k) > 0.5));
    if big.len() < 2 {
        return None;
    }
    // 核心之间至少隔开 4px（排除被整条分割线切开的同一张卡片）
    for (i, a) in big.iter().enumerate() {
        for o in &big[i + 1..] {
            let gx = o.x.saturating_sub(a.right()).max(a.x.saturating_sub(o.right()));
            let gy = o.y.saturating_sub(a.bottom()).max(a.y.saturating_sub(o.bottom()));
            if gx.max(gy) < 4 {
                return None;
            }
        }
    }
    let mut parts: Vec<Vec<u32>> = vec![Vec::new(); big.len()];
    for &i in &c.pix {
        let (x, y) = px_xy(cv, i);
        let d = |k: &Bounds| {
            let dx = (k.x as i64 - x as i64).max(x as i64 - k.right() as i64 + 1).max(0);
            let dy = (k.y as i64 - y as i64).max(y as i64 - k.bottom() as i64 + 1).max(0);
            dx * dx + dy * dy
        };
        let best = (0..big.len()).min_by_key(|&k| d(&big[k])).unwrap_or(0);
        parts[best].push(i);
    }
    Some(
        parts
            .into_iter()
            .filter(|p| p.len() >= 3)
            .map(|pix| {
                let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
                for &i in &pix {
                    let (x, y) = px_xy(cv, i);
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
                Comp { b: Bounds::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1), pix }
            })
            .collect(),
    )
}
fn expand(k: &Bounds, w: u32, limit: &Bounds) -> Bounds {
    let x0 = k.x.saturating_sub(w).max(limit.x);
    let y0 = k.y.saturating_sub(w).max(limit.y);
    let x1 = (k.right() + w).min(limit.right());
    let y1 = (k.bottom() + w).min(limit.bottom());
    Bounds::new(x0, y0, x1 - x0, y1 - y0)
}

fn side_bg(bg: &Bg, b: &Bounds, side: usize) -> Rgb {
    let (cx, cy) = (b.x + b.width / 2, b.y + b.height / 2);
    match side {
        0 => bg.at(cx, b.y.saturating_sub(1)),
        1 => bg.at(cx, b.bottom()),
        2 => bg.at(b.x.saturating_sub(1), cy),
        _ => bg.at(b.right(), cy),
    }
}

/// 有底色的色块（卡片、按钮、色条、头像底……），内容像素与它连成一块
/// 连通块的外圈是否闭合（四条边中段都有像素）：卡片里放了和页面底色相近的图片时，
/// 连通块中间是空的，但它仍是一张有底色的卡片
fn ring_closed(cv: &Canvas, c: &Comp) -> bool {
    let b = c.b;
    if b.width < 24 || b.height < 24 {
        return false;
    }
    let m = Members::new(cv, c);
    (0..4).all(|side| {
        let len = if side < 2 { b.width } else { b.height };
        let (a, e) = (len * 3 / 10, len - len * 3 / 10);
        let reach = ((b.width.min(b.height) as f32 * 0.1).ceil() as u32).max(4);
        let hit = (a..e)
            .filter(|&t| {
                (0..reach).any(|d| {
                    let (x, y) = match side {
                        0 => (b.x + t, b.y + d),
                        1 => (b.x + t, b.bottom() - 1 - d),
                        2 => (b.x + d, b.y + t),
                        _ => (b.right() - 1 - d, b.y + t),
                    };
                    m.has(x, y)
                })
            })
            .count();
        hit as f32 >= 0.9 * (e - a) as f32
    })
}

fn try_filled_box(ctx: &Ctx, c: &Comp, area: &Bounds, bg: &Bg) -> Option<BoxHit> {
    let cv = ctx.cv;
    let b = c.b;
    if b.width < 6 || b.height < 6 {
        return None;
    }
    let dense = (c.pix.len() as f32) >= 0.55 * b.area() as f32;
    // 空心的只考虑比图标大的块（圆环图标、"口"字不算卡片）
    let big = b.width.min(b.height) as f32 > (0.15 * ctx.short).max(48.0);
    if !dense && !(big && ring_closed(cv, c)) {
        return None;
    }
    let (mode, _) = color::mode_color(c.pix.iter().map(|&i| cv.get(i % cv.w, i / cv.w)), 4)?;
    if let Some(hit) = filled_box_with(ctx, c, area, bg, mode, dense) {
        return Some(hit);
    }
    // 内容（如商品图）面积比卡片底色还大时，众数色是内容的颜色：改用外框内侧一圈的颜色再试一次
    let edge: Vec<Rgb> = cv.ring(&b, 3, 0.25).into_iter().map(|s| s.2).filter(|p| color::dist(*p, bg.at(b.x, b.y)) > ctx.thr).collect();
    let (em, share) = color::mode_color(edge.into_iter(), 4)?;
    if share >= 0.5 && color::dist(em, mode) > ctx.thr_core() {
        return filled_box_with(ctx, c, area, bg, em, dense);
    }
    None
}

fn filled_box_with(ctx: &Ctx, c: &Comp, area: &Bounds, bg: &Bg, mode: Rgb, dense: bool) -> Option<BoxHit> {
    let cv = ctx.cv;
    let b = c.b;
    let tc = ctx.thr_core();
    let mut fill = Bg::Solid(mode);
    let (mut k, mut kn) = core_of(cv, c, &fill, tc);
    let mut confidence = 0.8;
    let mut ftol = tc * 1.5;
    let solid_area = k.map(|k| k.area()).unwrap_or(0);
    if solid_area * 100 < b.area() * 85 {
        // 纯色只解释了一部分：试试线性渐变，覆盖更完整就用渐变
        let solid = (fill, k, kn);
        let inner = shrink(&b, 0.12);
        let step = (c.pix.len() / 4000).max(1);
        let samples: Vec<_> = c
            .pix
            .iter()
            .step_by(step)
            .map(|&i| px_xy(cv, i))
            .filter(|&(x, y)| x >= inner.x && y >= inner.y && x < inner.right() && y < inner.bottom())
            .map(|(x, y)| (x as f32, y as f32, cv.get(x, y)))
            .collect();
        // 残差太大说明不是干净的渐变（照片、纹理）
        let grad = canvas::fit(&samples, &b).filter(|f| matches!(f.bg, Bg::Linear { .. }) && f.inliers >= 0.6 && f.rms <= 12.0).and_then(|f| {
            // AI 生成的渐变往往不是严格线性（带高光），按拟合残差放宽
            let tol = (tc * 1.5).max(3.0 * f.rms).min(30.0);
            let (gk, gn) = core_of(cv, c, &f.bg, tol);
            gk.filter(|g| g.area() * 100 >= b.area() * 45 && g.area() > solid_area).map(|g| (f.bg, g, gn, tol))
        });
        match grad {
            Some((g, gk, gn, tol)) => {
                fill = g;
                k = Some(gk);
                kn = gn;
                ftol = tol;
                confidence = 0.6;
            }
            None => {
                if solid_area * 100 < b.area() * 45 {
                    return None;
                }
                (fill, k, kn) = solid;
            }
        }
    }
    let k = k?;
    let lt = ctx.line_thick();
    let (short, long) = (k.width.min(k.height), k.width.max(k.height));
    if short < 6 || kn < 36 || (short <= lt && long >= 6 * short) || (kn as f32) < if dense { 0.3 } else { 0.06 } * k.area() as f32 {
        return None;
    }
    // 至少三条边的内侧是填充色（排除粗体字等实心笔画；允许一侧有贴边内容）
    let good_sides = (0..4)
        .filter(|&side| {
            let len = if side < 2 { k.width } else { k.height };
            let (a, e) = (len * 3 / 10, len - len * 3 / 10);
            let n = e.saturating_sub(a).max(1);
            // 胶囊形 / 大圆角的两端是弧线：允许向内找一小段
            let reach = ((k.width.min(k.height) as f32 * 0.1).ceil() as u32).max(2);
            let ok = (a..e)
                .filter(|&t| {
                    (1..=reach).any(|d| {
                        let (x, y) = match side {
                            0 => (k.x + t, k.y + d),
                            1 => (k.x + t, k.bottom() - 1 - d),
                            2 => (k.x + d, k.y + t),
                            _ => (k.right() - 1 - d, k.y + t),
                        };
                        color::dist(cv.get(x, y), fill.at(x, y)) <= ftol
                    })
                })
                .count() as u32;
            ok as f32 >= 0.6 * n as f32
        })
        .count();
    if good_sides < 3 {
        return None;
    }

    // 色块外圈：描边 / 阴影 / 贴边的内容
    let profs: Vec<Vec<Rgb>> = (0..4).map(|s| side_profile(cv, &k, &b, s)).collect();
    let fill_edge = fill.at(k.x + k.width / 2, k.y + k.height / 2);
    let strokes: Vec<Option<(usize, Rgb)>> = (0..4).map(|s| stroke_on(&profs[s], side_bg(bg, &b, s), fill_edge, ctx.thr)).collect();
    let stroke = if strokes.iter().all(Option::is_some) {
        let ws: Vec<usize> = strokes.iter().map(|s| s.unwrap().0).collect();
        let (lo, hi) = (*ws.iter().min().unwrap(), *ws.iter().max().unwrap());
        (hi - lo <= 1).then(|| {
            let mut sorted = ws.clone();
            sorted.sort_unstable();
            let mut sum = [0f32; 3];
            for s in strokes.iter().flatten() {
                for i in 0..3 {
                    sum[i] += s.1[i] / 4.0;
                }
            }
            (sorted[2] as u32, sum)
        })
    } else {
        None
    };
    let w = stroke.map(|s| s.0).unwrap_or(0);
    let mut r = if w > 0 { expand(&k, w, &b) } else { k };
    let mut halo = false;
    for (s, prof) in profs.iter().enumerate() {
        let rest = &prof[(w as usize).min(prof.len())..];
        if rest.is_empty() {
            continue;
        }
        if is_halo(rest, side_bg(bg, &b, s)) {
            halo = true;
        } else if w == 0 && rest.len() >= 3 {
            // 内容贴边（如卡片顶部的通栏图片）：外框延伸到这一侧
            match s {
                0 => r = Bounds::new(r.x, b.y, r.width, r.bottom() - b.y),
                1 => r.height = b.bottom() - r.y,
                2 => r = Bounds::new(b.x, r.y, r.right() - b.x, r.height),
                _ => r.width = b.right() - r.x,
            }
        }
    }

    let mut node = Node::new(NodeType::Rect, r);
    node.confidence = confidence;
    node.style.fill = Some(fill.to_fill(&r));
    let outer = match stroke {
        Some((_, c)) => Bg::Solid(c),
        None => fill,
    };
    let radii = style::corner_radii(cv, &r, bg, &outer);
    node.style.corner_radius = radii.and_then(|r| style::radius_value(r, 1.0));
    if let Some((sw, sc)) = stroke {
        node.style.stroke = Some(Stroke { color: color::hex_f(sc), width: sw });
    }
    if halo && bg.is_solid() {
        node.style.shadow = style::estimate_shadow(cv, &r, area, bg);
    }
    let rad = node.style.corner_radius.as_ref().map(|c| c.max()).unwrap_or(0) as f32;
    let inset = w + 2;
    let inner = r.inset(inset).filter(|i| i.width >= 6 && i.height >= 6).map(|i| (i, fill, Shape { r: i, radius: (rad - inset as f32).max(0.0) }));
    Some(BoxHit { node, rect: r, inner, thr: (!fill.is_solid()).then_some(ftol) })
}
/// 与底色同色、只靠描边或阴影显出轮廓的色块（输入框、白底上的白卡片）：连通块是一个"环"
fn try_hollow(ctx: &Ctx, c: &Comp, area: &Bounds, bg: &Bg) -> Option<BoxHit> {
    let cv = ctx.cv;
    let b = c.b;
    if b.width < 16 || b.height < 16 {
        return None;
    }
    let m = Members::new(cv, c);
    // 四条边的中间段都几乎被环覆盖（圆角 / 胶囊形的两端是弧线，只看中间 40%，并放宽到弧线的偏移量）
    let reach = ((b.width.min(b.height) as f32 * 0.1).ceil() as u32).max(4);
    for side in 0..4 {
        let len = if side < 2 { b.width } else { b.height };
        let (a, e) = (len * 3 / 10, len - len * 3 / 10);
        let mut hit = 0;
        for t in a..e {
            let found = (0..reach).any(|d| {
                let (x, y) = match side {
                    0 => (b.x + t, b.y + d),
                    1 => (b.x + t, b.bottom() - 1 - d),
                    2 => (b.x + d, b.y + t),
                    _ => (b.right() - 1 - d, b.y + t),
                };
                m.has(x, y)
            });
            hit += found as u32;
        }
        if (hit as f32) < 0.85 * (e - a) as f32 {
            return None;
        }
    }
    let center = shrink(&b, 0.3);
    let inside = (center.y..center.bottom()).flat_map(|y| (center.x..center.right()).map(move |x| (x, y))).filter(|&(x, y)| m.has(x, y)).count();
    if inside as f32 > 0.2 * center.area() as f32 {
        return None;
    }
    // 每条边的环厚度（中间一段的中位数）
    let mut t = [0u32; 4];
    for (side, ts) in t.iter_mut().enumerate() {
        let len = if side < 2 { b.width } else { b.height };
        // 只取边的中间 20%，跳过弧线外侧的空白后数第一段连续像素
        let mut v: Vec<u32> = (len * 2 / 5..(len * 3 / 5).max(len * 2 / 5 + 1))
            .map(|p| {
                let max = if side < 2 { b.height / 2 } else { b.width / 2 };
                let at = |d: u32| {
                    let (x, y) = match side {
                        0 => (b.x + p, b.y + d),
                        1 => (b.x + p, b.bottom() - 1 - d),
                        2 => (b.x + d, b.y + p),
                        _ => (b.right() - 1 - d, b.y + p),
                    };
                    m.has(x, y)
                };
                let start = (0..max.min(reach)).find(|&d| at(d)).unwrap_or(max);
                (start..max).take_while(|&d| at(d)).count() as u32
            })
            .collect();
        if v.is_empty() {
            return None;
        }
        v.sort_unstable();
        *ts = v[v.len() / 2];
    }
    if t.contains(&0) {
        return None;
    }
    let fill = bg.at(b.x + b.width / 2, b.y + b.height / 2);
    let max_t = *t.iter().max().unwrap();
    // 细描边：相对尺寸很细（排除 O、D 这类大字号的字母）
    if max_t <= 4 && (max_t as f32) <= (0.06 * b.width.min(b.height) as f32).max(1.5) {
        // 描边：取环上像素的众数色
        let w = {
            let mut s = t;
            s.sort_unstable();
            s[1].max(1)
        };
        let ring: Vec<Rgb> = c.pix.iter().map(|&i| px_xy(cv, i)).filter(|&(x, y)| x < b.x + w || y < b.y + w || x >= b.right() - w || y >= b.bottom() - w).map(|(x, y)| cv.get(x, y)).collect();
        let (sc, share) = color::mode_color(ring.into_iter(), 4)?;
        if share < 0.3 || color::dist(sc, fill) < 2.0 * ctx.thr {
            return None;
        }
        let mut node = Node::new(NodeType::Rect, b);
        node.confidence = 0.75;
        node.style.fill = Some(Bg::Solid(fill).to_fill(&b));
        node.style.stroke = Some(Stroke { color: color::hex_f(sc), width: w });
        node.style.corner_radius = style::corner_radii(cv, &b, bg, &Bg::Solid(sc)).and_then(|r| style::radius_value(r, 1.0));
        let rad = node.style.corner_radius.as_ref().map(|c| c.max()).unwrap_or(0) as f32;
        let inset = w + 2;
        let inner = b.inset(inset).filter(|i| i.width >= 6 && i.height >= 6).map(|i| (i, *bg, Shape { r: i, radius: (rad - inset as f32).max(0.0) }));
        return Some(BoxHit { node, rect: b, inner, thr: None });
    }
    // 阴影：环的内边缘就是卡片边缘
    if b.width <= t[2] + t[3] + 8 || b.height <= t[0] + t[1] + 8 || !bg.is_solid() {
        return None;
    }
    if max_t <= 4 {
        return None;
    }
    // 阴影环：从卡片边缘向外是低饱和、渐淡的暗色（排除彩色圆环图标等）
    for side in 0..4usize {
        let mid = if side < 2 { b.x + b.width / 2 } else { b.y + b.height / 2 };
        let prof: Vec<Rgb> = (0..t[side])
            .rev()
            .map(|d| match side {
                0 => cv.get(mid, b.y + d),
                1 => cv.get(mid, b.bottom() - 1 - d),
                2 => cv.get(b.x + d, mid),
                _ => cv.get(b.right() - 1 - d, mid),
            })
            .collect();
        if !is_halo(&prof, side_bg(bg, &b, side)) {
            return None;
        }
    }
    let r = Bounds::new(b.x + t[2], b.y + t[0], b.width - t[2] - t[3], b.height - t[0] - t[1]);
    let shadow = style::estimate_shadow(cv, &r, area, bg)?;
    // 圆角：卡片角上被阴影填上的面积
    let s = r.width.min(r.height) / 2;
    let corner = |x0: u32, y0: u32| (y0..y0 + s).flat_map(|y| (x0..x0 + s).map(move |x| (x, y))).filter(|&(x, y)| m.has(x, y)).count() as f32;
    let radii = [corner(r.x, r.y), corner(r.right() - s, r.y), corner(r.right() - s, r.bottom() - s), corner(r.x, r.bottom() - s)].map(|a| (a / 0.2146).sqrt());
    let mut node = Node::new(NodeType::Rect, r);
    node.confidence = 0.55;
    node.style.fill = Some(Bg::Solid(fill).to_fill(&r));
    node.style.shadow = Some(shadow);
    node.style.corner_radius = style::radius_value(radii, 1.0);
    let rad = node.style.corner_radius.as_ref().map(|c| c.max()).unwrap_or(0) as f32;
    let inner = r.inset(2).filter(|i| i.width >= 6 && i.height >= 6).map(|i| (i, *bg, Shape { r: i, radius: (rad - 2.0).max(0.0) }));
    Some(BoxHit { node, rect: r, inner, thr: None })
}

/// 分析一个容器的内部，返回未分组的直接子元素
pub(crate) fn analyze_raw(ctx: &Ctx, area: Bounds, bg: &Bg, shape: Option<&Shape>, depth: usize) -> Vec<Node> {
    let mut queue: Vec<(Comp, bool)> = label(ctx, &area, bg, shape).into_iter().map(|c| (c, true)).collect();
    let mut hits: Vec<BoxHit> = Vec::new();
    let mut rest: Vec<Comp> = Vec::new();
    while let Some((c, splittable)) = queue.pop() {
        if splittable {
            if let Some(parts) = split_by_cores(ctx, &c) {
                queue.extend(parts.into_iter().map(|p| (p, false)));
                continue;
            }
        }
        let ratio = c.pix.len() as f32 / c.b.area() as f32;
        if ratio < 0.55 {
            if let Some(h) = try_hollow(ctx, &c, &area, bg) {
                hits.push(h);
                continue;
            }
        }
        rest.push(c);
    }
    // 空心色块内部的元素由递归负责
    let hollow: Vec<Bounds> = hits.iter().map(|h| h.rect).collect();
    rest.retain(|c| !hollow.iter().any(|r| r.contains_ratio(&c.b) >= 0.9 && r.area() > c.b.area()));
    let mut pieces: Vec<Piece> = Vec::new();
    let mut solids: Vec<(BoxHit, Comp)> = Vec::new();
    for c in rest {
        match try_filled_box(ctx, &c, &area, bg) {
            Some(h) => solids.push((h, c)),
            None => pieces.push(Piece::from_comp(ctx, c, bg)),
        }
    }
    // 小实心块其实是图标或文字的一部分：落在图标外接框里（圆环里的圆点）、与同色文字排在同一行（粗体"日"）、
    // 紧挨着同色的大图标（购物车的轮子）。这些块当作普通笔画交给合并
    let tmax = pieces::text_max_h(ctx);
    // 判断时的"同伴"：普通笔画，加上已经判定为笔画的小实心块（粗体数字 "2026" 里的 "0" 和 "2" 都可能是实心块，
    // 要一个带一个地连到文字行上），反复迭代直到稳定
    struct Cand {
        b: Bounds,
        color: [u8; 3],
    }
    let mut peers: Vec<Cand> = pieces.iter().filter(|p| !p.solid).map(|p| Cand { b: p.b, color: color::to_u8(p.color) }).collect();
    let mut pending: Vec<(BoxHit, Comp)> = solids;
    let mut demoted = Vec::new();
    loop {
        let mut changed = false;
        let mut keep = Vec::new();
        for (h, c) in pending {
            let hb = h.rect;
            let small = h.node.style.stroke.is_none() && h.node.style.shadow.is_none() && hb.width.max(hb.height) as f32 <= (0.15 * ctx.short).max(tmax * 1.5);
            let fill = h.node.style.fill.as_ref().and_then(|f| f.main_color()).unwrap_or([0, 0, 0]);
            // 笔画不会是填满外接框的实心矩形（那样的是并排的标签、色块）
            let glyph_like = (c.pix.len() as f32) < 0.85 * c.b.area() as f32;
            let belongs = small
                && peers.iter().any(|p| {
                    let de = color::delta_e(fill, p.color);
                    let inside = p.b.area() > hb.area() && p.b.contains_ratio(&hb) >= 0.9;
                    let (hs, hl) = (p.b.height.min(hb.height) as f32, p.b.height.max(hb.height) as f32);
                    let ov = pieces::v_overlap(&p.b, &hb) as f32;
                    let in_row = glyph_like && de < 25.0 && hl <= tmax && hl <= 1.6 * hs && ov >= 0.5 * hs && pieces::gap_x(&p.b, &hb) as f32 <= 0.6 * hl;
                    let big = p.b.width.max(p.b.height) as f32;
                    let gap = pieces::gap_x(&p.b, &hb).max(pieces::gap_y(&p.b, &hb)) as f32;
                    let cluster = de < 15.0 && (hb.width.max(hb.height) as f32) <= 0.5 * big && gap <= 0.25 * big;
                    inside || in_row || cluster
                });
            if belongs {
                peers.push(Cand { b: hb, color: fill });
                demoted.push(c);
                changed = true;
            } else {
                keep.push((h, c));
            }
        }
        pending = keep;
        if !changed || pending.is_empty() {
            break;
        }
    }
    // 一整行都是实心字（粗体标题）时没有普通笔画可依附：≥3 个高度相近、紧挨着排成一行的小实心块也是文字
    let glyph = |h: &BoxHit, c: &Comp| {
        let hb = h.rect;
        h.node.style.stroke.is_none() && h.node.style.shadow.is_none() && (hb.height as f32) <= tmax && (c.pix.len() as f32) < 0.85 * c.b.area() as f32
    };
    let n = pending.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    for i in 0..n {
        for j in i + 1..n {
            let ((hi, ci), (hj, cj)) = (&pending[i], &pending[j]);
            if !glyph(hi, ci) || !glyph(hj, cj) {
                continue;
            }
            let (a, b) = (hi.rect, hj.rect);
            let (hs, hl) = (a.height.min(b.height) as f32, a.height.max(b.height) as f32);
            let same_row = hl <= 1.4 * hs && pieces::v_overlap(&a, &b) as f32 >= 0.7 * hs && pieces::gap_x(&a, &b) as f32 <= 0.6 * hl;
            let fa = hi.node.style.fill.as_ref().and_then(|f| f.main_color()).unwrap_or([0; 3]);
            let fb = hj.node.style.fill.as_ref().and_then(|f| f.main_color()).unwrap_or([0; 3]);
            if same_row && color::delta_e(fa, fb) < 15.0 {
                let (ra, rb) = (root(&mut parent, i), root(&mut parent, j));
                parent[rb] = ra;
            }
        }
    }
    let mut size = vec![0usize; n];
    for i in 0..n {
        let r = root(&mut parent, i);
        size[r] += 1;
    }
    for (i, (h, c)) in pending.into_iter().enumerate() {
        if size[root(&mut parent, i)] >= 3 {
            demoted.push(c);
        } else {
            hits.push(h);
        }
    }
    for c in demoted {
        let mut p = Piece::from_comp(ctx, c, bg);
        p.solid = false;
        pieces.push(p);
    }
    // 落在色块范围内、但没和色块连在一起的元素（如卡片里与页面同底色的图片中的物体）由色块的递归负责
    let rects: Vec<Bounds> = hits.iter().map(|h| h.rect).collect();
    let inside = |b: &Bounds| rects.iter().any(|r| r.area() > b.area() && r.contains_ratio(b) >= 0.9);
    pieces.retain(|p| !inside(&p.b));
    hits.retain(|h| !inside(&h.rect));
    let mut nodes = Vec::with_capacity(hits.len() + pieces.len());
    for h in hits {
        let mut node = h.node;
        if let Some((inner, fill, shape)) = h.inner {
            if depth + 1 < ctx.max_depth {
                let sub = Ctx { thr: h.thr.unwrap_or(ctx.thr).max(ctx.thr), ..*ctx };
                let (kids, mode) = analyze_grouped(&sub, inner, &fill, Some(&shape), depth + 1);
                if !kids.is_empty() {
                    node.node_type = NodeType::Frame;
                    node.children = kids;
                    node.layout = Some(group::placeholder(mode));
                }
            }
        }
        collapse_photo(&mut node);
        collapse_icon(ctx, &mut node);
        if !bg.is_solid() && node.children.is_empty() {
            let (cx, cy) = node.bounds.center();
            let local = color::to_u8(bg.at(cx as u32, cy as u32));
            if node.style.fill.as_ref().and_then(|f| f.main_color()).map(|c| color::delta_e(c, local) < 12.0).unwrap_or(false) {
                continue;
            }
        }
        nodes.push(node);
    }
    let boxes: Vec<Bounds> = nodes.iter().map(|n| n.bounds).collect();
    let icons: Vec<Bounds> = nodes.iter().filter(|n| n.node_type == NodeType::Icon).map(|n| n.bounds).collect();
    let bgc = color::to_u8(bg.mean());
    for n in pieces::into_nodes(ctx, pieces, bg) {
        let c = n.text_style.as_ref().and_then(|t| color::parse_hex(&t.color)).or_else(|| n.style.fill.as_ref().and_then(|f| f.main_color()));
        // 渐变底上按所在位置的底色比较
        let (cx, cy) = n.bounds.center();
        let local = if bg.is_solid() { bgc } else { color::to_u8(bg.at(cx as u32, cy as u32)) };
        let faint = c.map(|c| color::delta_e(c, local) < if bg.is_solid() { 5.0 } else { 12.0 }).unwrap_or(false);
        // 与底色几乎一样的碎块（噪点、压在色块边缘的残留）丢掉
        let (nmin, nmax) = (n.bounds.width.min(n.bounds.height), n.bounds.width.max(n.bounds.height));
        let short_hairline = nmin <= 2 && (nmax as f32) < 0.2 * area.width.max(area.height) as f32;
        // 渐变底上的低对比碎块基本是拟合残差（高光、非线性过渡）
        if faint && (!bg.is_solid() || nmax < 24 || short_hairline || boxes.iter().any(|b| b.contains_ratio(&n.bounds) >= 0.9)) {
            continue;
        }
        // 叠在图标上的部件属于该图标
        if n.node_type != NodeType::Text && icons.iter().any(|b| b.contains_ratio(&n.bounds) >= 0.6) {
            continue;
        }
        nodes.push(n);
    }
    nodes
}

/// 接近方形、不大、里面没有文字的色块（App 图标、带底的线性图标、手机/锁形状）：整体当作一个图标
fn collapse_icon(ctx: &Ctx, node: &mut Node) {
    if node.children.is_empty() || node.style.stroke.is_some() {
        return;
    }
    let (w, h) = (node.bounds.width as f32, node.bounds.height as f32);
    // 明显偏宽的小色块是徽标 / 标签（如「01」），里面是文字，不是图标
    if w.max(h) > 0.3 * ctx.short || !(0.6..=1.25).contains(&(w / h)) {
        return;
    }
    // 看得清的文字（与底色对比明显）才算
    let fill = node.style.fill.as_ref().and_then(|f| f.main_color());
    let mut has_text = false;
    node.walk(&mut |d, _| {
        if let Some(t) = d.text_style.as_ref().filter(|t| t.font_size >= 10) {
            let de = match (fill, color::parse_hex(&t.color)) {
                (Some(f), Some(c)) => color::delta_e(f, c),
                _ => 100.0,
            };
            has_text |= de >= 30.0;
        }
    });
    if has_text {
        return;
    }
    node.node_type = NodeType::Icon;
    node.children.clear();
    node.layout = None;
    node.style.shadow = None;
    node.confidence = 0.55;
}

/// 色块里只有一堆与填充色相近的低对比碎块：其实是照片 / 插画，整块当作 image
fn collapse_photo(node: &mut Node) {
    let Some(fill) = node.style.fill.as_ref().and_then(|f| f.main_color()) else { return };
    let mut total = 0usize;
    let mut faint = 0usize;
    let mut readable_text = false;
    for c in &node.children {
        c.walk(&mut |d, _| {
            total += 1;
            let col = d.text_style.as_ref().and_then(|t| color::parse_hex(&t.color)).or_else(|| d.style.fill.as_ref().and_then(|f| f.main_color()));
            let de = col.map(|c| color::delta_e(c, fill));
            if de.map(|d| d < 12.0).unwrap_or(d.node_type == NodeType::Image) {
                faint += 1;
            }
            if d.node_type == NodeType::Text && de.unwrap_or(0.0) >= 30.0 && d.text_style.as_ref().map(|t| t.font_size >= 10).unwrap_or(false) {
                readable_text = true;
            }
        });
    }
    // 有清晰文字的（如渐变按钮上的"登录"）是真正的色块：只去掉低对比的叶子碎块
    if readable_text {
        node.children.retain(|c| {
            if !c.children.is_empty() || c.node_type == NodeType::Image {
                return true;
            }
            let col = c.text_style.as_ref().and_then(|t| color::parse_hex(&t.color)).or_else(|| c.style.fill.as_ref().and_then(|f| f.main_color()));
            col.map(|x| color::delta_e(x, fill) >= 12.0).unwrap_or(true)
        });
        return;
    }
    let mut any_text = false;
    node.walk(&mut |d, _| any_text |= d.node_type == NodeType::Text);
    let big = node.bounds.width.min(node.bounds.height) >= 80;
    if (total >= 6 || (total >= 2 && big && !any_text)) && faint * 10 >= total * 6 {
        node.node_type = NodeType::Image;
        node.children.clear();
        node.layout = None;
        node.style.fill = None;
        node.style.stroke = None;
        node.confidence = 0.5;
    }
}

/// 分析并按 XY-cut 分组
pub(crate) fn analyze_grouped(ctx: &Ctx, area: Bounds, bg: &Bg, shape: Option<&Shape>, depth: usize) -> (Vec<Node>, LayoutMode) {
    group::xy_cut(analyze_raw(ctx, area, bg, shape, depth))
}
