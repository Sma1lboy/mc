//! 实例截图浏览 —— 枚举 `screenshots/` 下的图片、按需读出为 data URL、删除。
//!
//! 设计要点:
//! - **列表只给元数据**(文件名 / 大小 / 修改时间),不内联图片字节 —— 截图常是 1~3 MB 的
//!   全分辨率 PNG,一次性 base64 几十张会让 IPC 负载与内存爆掉。图片改由 [`read_screenshot`]
//!   按需逐张读取(UI 滚动到哪张才取哪张)。
//! - 列表按**修改时间倒序**(最新截图在前),贴合"刚截的图最想看"的直觉。
//! - 复用 [`super::base64_encode`] / [`super::sniff_image_mime`](图标探测同款),避免为
//!   "把图喂给 webview"再引入 asset 协议配置或额外依赖。
//! - 所有按 `file_name` 定位的操作都拒绝路径分隔符 / `..`,防止越权读到实例目录之外。

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{CoreError, IoResultExt, Result};
use crate::instance::Instance;

/// 单张截图的列表视图(不含图片字节)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScreenshotInfo {
    /// 磁盘文件名(启停/删除/读取的稳定标识)。
    pub file_name: String,
    /// 文件字节大小。
    pub size: u64,
    /// 修改时间(epoch 毫秒;取不到为 0)。用于倒序排列。
    pub modified: u64,
}

/// 单张图片内联上限:超过则 [`read_screenshot`] 拒绝(防异常大文件塞爆 IPC)。
const MAX_SCREENSHOT_BYTES: u64 = 24 * 1024 * 1024;

/// 是否为我们识别的截图文件:`.png` / `.jpg` / `.jpeg`(忽略大小写)。
fn is_image(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg")
}

/// 校验 `file_name` 是单一路径段(不含分隔符、不是 `.`/`..`),防穿越;通过返回安全绝对路径。
fn resolve_in_dir(dir: &Path, file_name: &str) -> Result<PathBuf> {
    if file_name.is_empty()
        || file_name == "."
        || file_name == ".."
        || file_name.contains('/')
        || file_name.contains('\\')
    {
        return Err(CoreError::other(format!("非法截图文件名: {file_name}")));
    }
    Ok(dir.join(file_name))
}

/// 取文件修改时间(epoch 毫秒),失败返回 0。
fn modified_millis(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 列出实例 `screenshots/` 下的所有截图(仅元数据),按修改时间倒序(最新在前)。
///
/// 目录不存在(从未截图)返回空列表,不报错。
pub fn list_screenshots(inst: &Instance) -> Vec<ScreenshotInfo> {
    let dir = inst.screenshots_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut out: Vec<ScreenshotInfo> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        if !is_image(&file_name) {
            continue;
        }
        let meta = match std::fs::metadata(&path) {
            Ok(m) if m.is_file() => m,
            _ => continue,
        };
        out.push(ScreenshotInfo {
            file_name,
            size: meta.len(),
            modified: modified_millis(&meta),
        });
    }

    // 修改时间倒序;同时间再按文件名兜底,保证顺序确定。
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.file_name.cmp(&b.file_name)));
    out
}

/// 按需读取一张截图,编码为 `data:` URL(mime 按内容嗅探)。
///
/// 校验:文件名为单一路径段、确是受支持图片、不超过 [`MAX_SCREENSHOT_BYTES`]。
pub fn read_screenshot(inst: &Instance, file_name: &str) -> Result<String> {
    if !is_image(file_name) {
        return Err(CoreError::other("不是受支持的截图格式"));
    }
    let path = resolve_in_dir(&inst.screenshots_dir(), file_name)?;
    let meta = std::fs::metadata(&path).with_path(&path)?;
    if meta.len() > MAX_SCREENSHOT_BYTES {
        return Err(CoreError::other("截图过大,无法预览"));
    }
    let bytes = std::fs::read(&path).with_path(&path)?;
    Ok(format!(
        "data:{};base64,{}",
        super::sniff_image_mime(&bytes),
        super::base64_encode(&bytes)
    ))
}

/// 删除一张截图(优先移入系统回收站,可找回;回收站不可用时回退永久删除)。
pub fn delete_screenshot(inst: &Instance, file_name: &str) -> Result<()> {
    let path = resolve_in_dir(&inst.screenshots_dir(), file_name)?;
    if !path.exists() {
        return Ok(()); // 幂等。
    }
    if trash::delete(&path).is_ok() {
        return Ok(());
    }
    std::fs::remove_file(&path).with_path(&path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    struct TempInst {
        root: PathBuf,
        inst: Instance,
    }

    impl TempInst {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir()
                .join(format!("mc-core-shots-test-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            let inst = Instance::new("1.20.1", root.clone());
            fs::create_dir_all(inst.screenshots_dir()).unwrap();
            Self { root, inst }
        }
    }

    impl Drop for TempInst {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// 一个最小 PNG(8 字节签名)即可被 is_image + sniff 识别为 image/png。
    const PNG_SIG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

    #[test]
    fn lists_only_images_newest_first() {
        let t = TempInst::new("list");
        let dir = t.inst.screenshots_dir();
        fs::write(dir.join("a.png"), PNG_SIG).unwrap();
        fs::write(dir.join("b.jpg"), PNG_SIG).unwrap();
        fs::write(dir.join("notes.txt"), b"ignore me").unwrap();

        // 让 b 比 a 新:重写 b 以更新 mtime(粗略但足够;若同毫秒则按文件名兜底)。
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(dir.join("b.jpg"), PNG_SIG).unwrap();

        let shots = list_screenshots(&t.inst);
        assert_eq!(shots.len(), 2, "txt 应被忽略");
        // 倒序:更晚写的 b.jpg 在前。
        assert_eq!(shots[0].file_name, "b.jpg");
        assert_eq!(shots[1].file_name, "a.png");
        assert!(shots.iter().all(|s| s.size > 0));
    }

    #[test]
    fn read_screenshot_returns_data_url() {
        let t = TempInst::new("read");
        fs::write(t.inst.screenshots_dir().join("shot.png"), PNG_SIG).unwrap();
        let url = read_screenshot(&t.inst, "shot.png").unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn read_screenshot_rejects_traversal_and_nonimage() {
        let t = TempInst::new("guard");
        assert!(read_screenshot(&t.inst, "../secret.png").is_err());
        assert!(read_screenshot(&t.inst, "notes.txt").is_err());
    }

    #[test]
    fn delete_screenshot_is_idempotent() {
        let t = TempInst::new("del");
        let p = t.inst.screenshots_dir().join("x.png");
        fs::write(&p, PNG_SIG).unwrap();
        delete_screenshot(&t.inst, "x.png").unwrap();
        assert!(!p.exists());
        // 再删一次:文件已不存在,应幂等成功。
        delete_screenshot(&t.inst, "x.png").unwrap();
    }
}
