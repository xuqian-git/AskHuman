//! 图片附件落盘：sanitize 文件名、media_type→扩展名、base64 解码。

use crate::models::ImageAttachment;
use crate::paths;
use anyhow::{Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use std::path::Path;

/// 把所有图片落盘到 `temp/humaninloop/<request_id>/`，返回绝对路径列表。
pub fn save(images: &[ImageAttachment], request_id: &str) -> Result<Vec<String>> {
    if images.is_empty() {
        return Ok(Vec::new());
    }
    let dir = paths::request_temp_dir(request_id);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("创建图片目录失败: {}", dir.display()))?;

    let mut paths_out = Vec::with_capacity(images.len());
    for (index, img) in images.iter().enumerate() {
        paths_out.push(save_one(img, index, &dir)?);
    }
    Ok(paths_out)
}

fn save_one(img: &ImageAttachment, index: usize, dir: &Path) -> Result<String> {
    let ext = extension_from_media_type(&img.media_type);
    let filename = match img.filename.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(name) => sanitize_filename(name, ext),
        None => format!("img-{}.{}", index + 1, ext),
    };
    let file_path = dir.join(&filename);
    let data = decode_image_data(&img.data)?;
    std::fs::write(&file_path, &data)
        .with_context(|| format!("写入图片失败: {}", file_path.display()))?;
    Ok(file_path.to_string_lossy().to_string())
}

fn extension_from_media_type(media_type: &str) -> &'static str {
    match media_type.to_ascii_lowercase().as_str() {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/svg+xml" => "svg",
        _ => "bin",
    }
}

/// 去掉路径分隔符等危险字符，确保文件落在目标目录内。
fn sanitize_filename(raw: &str, fallback_ext: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    let cleaned: String = base
        .chars()
        .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\0'))
        .collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() {
        format!("img.{}", fallback_ext)
    } else {
        trimmed.to_string()
    }
}

/// 解码 base64，兼容含 `data:...;base64,` 前缀的内嵌格式。
fn decode_image_data(data: &str) -> Result<Vec<u8>> {
    let payload = match data.find("base64,") {
        Some(idx) => &data[idx + "base64,".len()..],
        None => data,
    };
    let cleaned: String = payload.chars().filter(|c| !c.is_whitespace()).collect();
    B64.decode(cleaned.as_bytes())
        .context("图片 base64 解码失败")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_path_separators() {
        assert_eq!(sanitize_filename("/etc/passwd", "png"), "passwd");
        assert_eq!(sanitize_filename("../../foo.png", "png"), "foo.png");
        assert_eq!(sanitize_filename("a\\b\\c.jpg", "png"), "c.jpg");
        assert_eq!(sanitize_filename("", "png"), "img.png");
        assert_eq!(sanitize_filename(".....", "png"), "img.png");
    }

    #[test]
    fn extension_mapping() {
        assert_eq!(extension_from_media_type("image/png"), "png");
        assert_eq!(extension_from_media_type("image/JPEG"), "jpg");
        assert_eq!(extension_from_media_type("image/svg+xml"), "svg");
        assert_eq!(extension_from_media_type("application/unknown"), "bin");
    }

    #[test]
    fn decode_handles_data_uri() {
        let png_b64 = "iVBORw0KGgo=";
        let bytes = decode_image_data(&format!("data:image/png;base64,{}", png_b64)).unwrap();
        assert_eq!(bytes, B64.decode(png_b64).unwrap());
    }

    #[test]
    fn decode_handles_whitespace() {
        let bytes = decode_image_data("aGVs\nbG8=").unwrap();
        assert_eq!(bytes, b"hello");
    }
}
