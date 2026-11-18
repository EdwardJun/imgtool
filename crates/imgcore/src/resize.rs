//! 带透明通道的缩放。
//!
//! `image` 的 `resize_exact` 直接对 RGBA 四个通道分别插值，完全透明像素里藏着的颜色
//! （例如去背景后保留的白色）会被混进边缘，缩小后的图标在深色背景上出现一圈白边。
//! 这里先预乘 alpha 再插值，透明像素的颜色就不再参与混合。

use image::{imageops::FilterType, DynamicImage, ImageBuffer, Rgba};

/// 缩放到精确尺寸；有半透明 / 透明像素时按预乘 alpha 插值，全不透明时与 `resize_exact` 相同
pub fn resize_exact(img: &DynamicImage, width: u32, height: u32, filter: FilterType) -> DynamicImage {
    if !has_transparency(img) {
        return img.resize_exact(width, height, filter);
    }
    let src = img.to_rgba8();
    let premul: ImageBuffer<Rgba<u16>, Vec<u16>> = ImageBuffer::from_fn(src.width(), src.height(), |x, y| {
        let [r, g, b, a] = src.get_pixel(x, y).0;
        let a16 = a as u32;
        // 8 位颜色扩到 16 位（×257）再乘 alpha/255，保留精度
        let p = |c: u8| ((c as u32 * 257 * a16 + 127) / 255) as u16;
        Rgba([p(r), p(g), p(b), a as u16 * 257])
    });
    let scaled = image::imageops::resize(&premul, width, height, filter);
    let out = ImageBuffer::from_fn(width, height, |x, y| {
        let [r, g, b, a] = scaled.get_pixel(x, y).0;
        if a == 0 {
            return Rgba([0, 0, 0, 0]);
        }
        let a32 = a as u32;
        // 预乘值 = 颜色16 × alpha16 / 65535，反算回 16 位颜色再降到 8 位；
        // 插值有轻微振铃，颜色可能略大于 alpha，按上限截断
        let u = |c: u16| ((((c as u32 * 65535 + a32 / 2) / a32).min(65535) + 128) / 257) as u8;
        Rgba([u(r), u(g), u(b), ((a32 + 128) / 257) as u8])
    });
    DynamicImage::ImageRgba8(out)
}

fn has_transparency(img: &DynamicImage) -> bool {
    if !img.color().has_alpha() {
        return false;
    }
    match img {
        DynamicImage::ImageRgba8(buf) => buf.pixels().any(|p| p.0[3] != 255),
        DynamicImage::ImageLumaA8(buf) => buf.pixels().any(|p| p.0[1] != 255),
        // 16 位 / 浮点等少见格式直接按有透明处理
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    /// 左半透明白色（alpha=0），右半不透明蓝色：缩小后边缘不应混入白色
    #[test]
    fn transparent_color_does_not_bleed() {
        let img = RgbaImage::from_fn(64, 64, |x, _| if x < 32 { Rgba([255, 255, 255, 0]) } else { Rgba([60, 70, 230, 255]) });
        let out = resize_exact(&DynamicImage::ImageRgba8(img.clone()), 16, 16, FilterType::Lanczos3).to_rgba8();
        for p in out.pixels().filter(|p| p.0[3] > 20) {
            let [r, g, b, _] = p.0;
            // Lanczos 在边缘本身有几个色阶的振铃，允许小误差；混入白色会偏差上百
            assert!(r.abs_diff(60) <= 8 && g.abs_diff(70) <= 8 && b.abs_diff(230) <= 8, "边缘颜色应保持蓝色: {:?}", p.0);
        }
        // 不透明区域颜色和 alpha 原样保留
        assert_eq!(out.get_pixel(14, 8).0, [60, 70, 230, 255]);
        // 半透明的同色像素缩放后颜色不变
        let half = RgbaImage::from_pixel(8, 8, Rgba([200, 100, 50, 128]));
        let half = resize_exact(&DynamicImage::ImageRgba8(half), 4, 4, FilterType::Lanczos3).to_rgba8();
        assert!(half.pixels().all(|p| p.0[0].abs_diff(200) <= 1 && p.0[1].abs_diff(100) <= 1 && p.0[2].abs_diff(50) <= 1 && p.0[3].abs_diff(128) <= 1), "{:?}", half.get_pixel(0, 0));
        // 对照：不预乘时会出现发白的半透明边
        let naive = DynamicImage::ImageRgba8(img).resize_exact(16, 16, FilterType::Lanczos3).to_rgba8();
        assert!(naive.pixels().any(|p| p.0[3] > 0 && p.0[0] > 120));
    }

    #[test]
    fn opaque_images_unchanged_path() {
        let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(10, 10, Rgba([10, 20, 30, 255])));
        let out = resize_exact(&img, 5, 5, FilterType::Lanczos3).to_rgba8();
        assert!(out.pixels().all(|p| p.0 == [10, 20, 30, 255]));
    }
}
