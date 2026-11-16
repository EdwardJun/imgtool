//! 本地参考图 → hfsyapi 图床链接：上传前压缩，链接按"接口地址 + 文件路径 + 修改时间 + 大小"缓存，
//! 快过期（剩余不足 30 分钟）时重新上传。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use serde::Serialize;
use tauri::State;

use imggen::upload::now_secs;
use imggen::{supports_upload, ImageUploader, UploadedImage};

/// 剩余有效期不足这个时长就重新上传
pub const REFRESH_MARGIN_SECS: u64 = 30 * 60;

#[derive(Default)]
pub struct RefCache(pub Mutex<HashMap<String, UploadedImage>>);

impl RefCache {
    pub fn get(&self, key: &str, now: u64) -> Option<UploadedImage> {
        let map = self.0.lock().ok()?;
        map.get(key).filter(|u| u.expires_at > now + REFRESH_MARGIN_SECS).cloned()
    }
    pub fn put(&self, key: String, value: UploadedImage) {
        if let Ok(mut map) = self.0.lock() {
            map.insert(key, value);
        }
    }
}

pub fn cache_key(base_url: &str, path: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("读取参考图 {} 失败: {e}", path.display()))?;
    let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis()).unwrap_or(0);
    Ok(format!("{}|{}|{}|{}", base_url.trim().trim_end_matches('/').to_ascii_lowercase(), path.display(), mtime, meta.len()))
}

pub use imgcore::postprocess::prepare_reference;

#[derive(Debug, Serialize)]
pub struct UploadedRef {
    pub path: String,
    pub url: String,
    pub expires_at: u64,
    pub bytes: u64,
    /// 命中缓存，没有重新上传
    pub cached: bool,
    pub compressed: bool,
}

/// 上传单张本地参考图（命中缓存则直接返回）
pub async fn upload_one(cache: &RefCache, base_url: &str, api_key: &str, path: &str) -> Result<UploadedRef, String> {
    let p = Path::new(path);
    let key = cache_key(base_url, p)?;
    if let Some(hit) = cache.get(&key, now_secs()) {
        return Ok(UploadedRef { path: path.into(), url: hit.url, expires_at: hit.expires_at, bytes: hit.size, cached: true, compressed: false });
    }
    let owned = p.to_path_buf();
    let prepared = tauri::async_runtime::spawn_blocking(move || prepare_reference(&owned)).await.map_err(|e| e.to_string())??;
    let uploader = ImageUploader::new(base_url, api_key).map_err(|e| e.to_string())?;
    let up = uploader.upload(&prepared.data, prepared.mime, &prepared.filename).await.map_err(|e| e.to_string())?;
    cache.put(key, up.clone());
    Ok(UploadedRef { path: path.into(), url: up.url, expires_at: up.expires_at, bytes: up.size, cached: false, compressed: prepared.compressed })
}

pub fn is_remote(r: &str) -> bool {
    let r = r.trim();
    r.starts_with("http://") || r.starts_with("https://") || r.starts_with("data:")
}

/// 生成前把参考图里的本地路径换成图床链接；不支持上传的接口原样返回（由生成接口按 data URL 发送）
pub async fn resolve_references(cache: &RefCache, base_url: &str, api_key: &str, refs: &[String]) -> Result<Vec<String>, String> {
    if !supports_upload(base_url) {
        return Ok(refs.to_vec());
    }
    let mut out = Vec::with_capacity(refs.len());
    for (i, r) in refs.iter().enumerate() {
        if is_remote(r) {
            out.push(r.clone());
        } else {
            let up = upload_one(cache, base_url, api_key, r).await.map_err(|e| format!("第 {} 张参考图上传失败：{e}", i + 1))?;
            out.push(up.url);
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn upload_reference(cache: State<'_, RefCache>, base_url: String, api_key: String, path: String) -> Result<UploadedRef, String> {
    if !supports_upload(&base_url) {
        return Err("当前接口不支持图床上传".into());
    }
    upload_one(&cache, &base_url, &api_key, &path).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn save(path: &Path, w: u32, h: u32, alpha: bool) {
        let mut seed = 3u32;
        let img = image::RgbaImage::from_fn(w, h, |x, y| {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            let a = if alpha && x < w / 4 { 0 } else { 255 };
            image::Rgba([(x % 256) as u8, (y % 256) as u8, (seed >> 16) as u8, a])
        });
        image::DynamicImage::ImageRgba8(img).save(path).unwrap();
    }

    #[test]
    fn small_files_pass_through() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.png");
        save(&p, 64, 64, false);
        let r = prepare_reference(&p).unwrap();
        assert!(!r.compressed);
        assert_eq!((r.mime, r.filename.as_str()), ("image/png", "a.png"));
        assert_eq!(r.data, std::fs::read(&p).unwrap());
    }

    #[test]
    fn large_opaque_becomes_jpeg_within_limit() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big.png");
        save(&p, 3000, 1500, false);
        let r = prepare_reference(&p).unwrap();
        assert!(r.compressed);
        assert_eq!((r.mime, r.filename.as_str()), ("image/jpeg", "big.jpg"));
        let img = image::load_from_memory(&r.data).unwrap();
        assert_eq!((img.width(), img.height()), (2048, 1024));
    }

    #[test]
    fn large_transparent_stays_png() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("logo.png");
        save(&p, 2500, 2500, true);
        let r = prepare_reference(&p).unwrap();
        assert_eq!(r.mime, "image/png");
        let img = image::load_from_memory(&r.data).unwrap().to_rgba8();
        assert_eq!(img.width(), 2048);
        assert_eq!(img.get_pixel(0, 0)[3], 0, "透明区域保留");
    }

    #[test]
    fn cache_respects_expiry_margin_and_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.png");
        save(&p, 16, 16, false);
        let cache = RefCache::default();
        let key = cache_key("https://www.hfsyapi.cn/", &p).unwrap();
        assert_eq!(key, cache_key("https://WWW.hfsyapi.cn", &p).unwrap(), "地址大小写/末尾斜杠不影响");
        cache.put(key.clone(), UploadedImage { url: "u".into(), expires_at: 10_000, size: 1 });
        assert!(cache.get(&key, 10_000 - REFRESH_MARGIN_SECS - 1).is_some());
        assert!(cache.get(&key, 10_000 - REFRESH_MARGIN_SECS).is_none(), "快过期时重新上传");
        // 文件内容变了（大小不同）→ key 不同
        std::thread::sleep(std::time::Duration::from_millis(5));
        save(&p, 32, 32, false);
        assert_ne!(key, cache_key("https://www.hfsyapi.cn", &p).unwrap());
    }

    /// 真实接口联调（会扣一次生成费用）：
    /// HFSYAPI_API_KEY=sk-... cargo test -p imgtool-desktop live_ -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_upload_then_generate() {
        let key = std::env::var("HFSYAPI_API_KEY").expect("需要 HFSYAPI_API_KEY");
        let base = std::env::var("HFSYAPI_BASE_URL").unwrap_or_else(|_| "https://www.hfsyapi.cn".into());
        let dir = tempfile::tempdir().unwrap();
        // 3000x1500 的大图：会先压缩到 2048 再上传
        let p = dir.path().join("big-ref.png");
        let img = image::RgbImage::from_fn(3000, 1500, |x, y| {
            let c = if ((x / 150) + (y / 150)) % 2 == 0 { [230, 120, 40] } else { [40, 90, 200] };
            image::Rgb(c)
        });
        img.save(&p).unwrap();
        let path = p.display().to_string();
        let cache = RefCache::default();
        tauri::async_runtime::block_on(async {
            let t = std::time::Instant::now();
            let first = upload_one(&cache, &base, &key, &path).await.expect("上传失败");
            println!("upload: {:.1}s compressed={} bytes={} expires_in={}h url_len={}", t.elapsed().as_secs_f32(), first.compressed, first.bytes, (first.expires_at - now_secs()) / 3600, first.url.len());
            assert!(first.compressed && !first.cached);
            let second = upload_one(&cache, &base, &key, &path).await.unwrap();
            assert!(second.cached && second.url == first.url, "第二次应命中缓存");

            let refs = resolve_references(&cache, &base, &key, std::slice::from_ref(&path)).await.unwrap();
            assert_eq!(refs, vec![first.url.clone()]);
            let provider = imggen::TaskApiProvider::new(imggen::TaskApiConfig { base_url: base.clone(), api_key: key.clone(), model: "gpt-image-2".into() }).unwrap();
            let req = imggen::GenerateRequest {
                prompt: "参考图是橙蓝相间的棋盘格，请画一个使用同样配色的简洁图标".into(),
                width: Some(1024),
                height: Some(1024),
                reference_images: refs,
                ..Default::default()
            };
            let t = std::time::Instant::now();
            let (id, images) = provider.run_once(&req).await.expect("生成失败");
            println!("generate: {:.1}s task={id} bytes={}", t.elapsed().as_secs_f32(), images[0].data.len());
            std::fs::write("/private/tmp/live-upload-result.png", &images[0].data).unwrap();
        });
    }

    #[test]
    fn non_upload_providers_keep_local_paths() {
        let cache = RefCache::default();
        let refs = vec!["/tmp/a.png".to_string(), "https://x/b.png".to_string()];
        let out = tauri::async_runtime::block_on(resolve_references(&cache, "https://api.openai.com", "k", &refs)).unwrap();
        assert_eq!(out, refs);
    }
}
