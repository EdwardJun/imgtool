//! 路径边界：所有读写都限制在项目根目录（MCP Roots / --root / cwd）之内

use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Workspace {
    roots: Vec<PathBuf>,
}

impl Workspace {
    /// roots 会被规范化，不存在的目录会被丢弃
    pub fn new(roots: impl IntoIterator<Item = PathBuf>) -> Result<Self, String> {
        let mut out: Vec<PathBuf> = Vec::new();
        for r in roots {
            if let Ok(c) = r.canonicalize() {
                if c.is_dir() && !out.contains(&c) {
                    out.push(c);
                }
            }
        }
        if out.is_empty() {
            return Err("没有可用的项目根目录".into());
        }
        Ok(Self { roots: out })
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    pub fn primary(&self) -> &Path {
        &self.roots[0]
    }

    /// 返回包含 path 的根目录
    pub fn root_of(&self, path: &Path) -> Option<&Path> {
        self.roots.iter().filter(|r| path.starts_with(r)).max_by_key(|r| r.as_os_str().len()).map(|r| r.as_path())
    }

    fn join(&self, input: &str) -> PathBuf {
        let p = Path::new(input.trim());
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.primary().join(p)
        }
    }

    /// 解析已存在的文件/目录，并确认位于根目录内
    pub fn resolve_existing(&self, input: &str) -> Result<PathBuf, String> {
        let joined = self.join(input);
        let canonical = joined.canonicalize().map_err(|_| format!("路径不存在: {input}"))?;
        self.ensure_inside(&canonical, input)?;
        Ok(canonical)
    }

    /// 解析输出路径（可以不存在），通过规范化最近的已存在祖先目录来校验
    pub fn resolve_output(&self, input: &str) -> Result<PathBuf, String> {
        let joined = self.join(input);
        if joined.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(format!("输出路径不允许包含 ..: {input}"));
        }
        let mut existing = joined.as_path();
        let mut rest = Vec::new();
        while !existing.exists() {
            rest.push(existing.file_name().ok_or_else(|| format!("无效路径: {input}"))?.to_owned());
            existing = existing.parent().ok_or_else(|| format!("无效路径: {input}"))?;
        }
        let mut canonical = existing.canonicalize().map_err(|e| e.to_string())?;
        for part in rest.into_iter().rev() {
            canonical.push(part);
        }
        self.ensure_inside(&canonical, input)?;
        Ok(canonical)
    }

    fn ensure_inside(&self, path: &Path, input: &str) -> Result<(), String> {
        if self.root_of(path).is_some() {
            Ok(())
        } else {
            Err(format!(
                "路径 {input} 不在项目根目录内（允许: {}）",
                self.roots.iter().map(|r| r.display().to_string()).collect::<Vec<_>>().join(", ")
            ))
        }
    }

    /// 显示用：根目录内的路径转为相对路径
    pub fn relative(&self, path: &Path) -> String {
        match self.root_of(path) {
            Some(root) if self.roots.len() == 1 => path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/"),
            _ => path.display().to_string(),
        }
    }
}

/// 预设里的 dir 只允许项目内的相对路径
pub fn validate_relative_dir(dir: &str) -> Result<(), String> {
    let p = Path::new(dir);
    if p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(format!("输出目录必须是项目内的相对路径: {dir}"));
    }
    Ok(())
}

/// file:// URI 转本地路径（处理百分号编码与 Windows 盘符）
pub fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let bytes = rest.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(b) = u8::from_str_radix(hex, 16) {
                decoded.push(b);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    let s = String::from_utf8(decoded).ok()?;
    // file:///C:/x → C:/x
    let s = if s.len() >= 3 && s.as_bytes()[0] == b'/' && s.as_bytes()[2] == b':' { s[1..].to_string() } else { s };
    Some(PathBuf::from(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_decoding() {
        assert_eq!(file_uri_to_path("file:///Users/a/My%20Proj"), Some(PathBuf::from("/Users/a/My Proj")));
        assert_eq!(file_uri_to_path("file:///C:/work/x"), Some(PathBuf::from("C:/work/x")));
        assert_eq!(file_uri_to_path("file:///a/%E4%B8%AD"), Some(PathBuf::from("/a/中")));
        assert_eq!(file_uri_to_path("https://x"), None);
    }

    #[test]
    fn boundary_checks() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        std::fs::create_dir_all(root.join("img")).unwrap();
        std::fs::write(root.join("img/a.png"), b"x").unwrap();
        std::fs::write(dir.path().join("secret.png"), b"x").unwrap();
        let ws = Workspace::new([root.clone()]).unwrap();

        assert!(ws.resolve_existing("img/a.png").is_ok());
        assert!(ws.resolve_existing("../secret.png").is_err());
        assert!(ws.resolve_existing(dir.path().join("secret.png").to_str().unwrap()).is_err());
        assert!(ws.resolve_output("img/new/b.webp").is_ok());
        assert!(ws.resolve_output("../b.webp").is_err());
        assert_eq!(ws.relative(&ws.resolve_existing("img/a.png").unwrap()), "img/a.png");
    }
}
