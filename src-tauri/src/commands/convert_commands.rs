//! Convert selected files: images (resize / change format) and office
//! documents (LibreOffice), through `convert_engine`. Outputs go next to
//! the inputs (or into a chosen folder) under free names; originals stay.

use crate::convert_engine::{self, ImageSpec};
use crate::core::error::AppError;
use serde::Serialize;
use std::path::{Path, PathBuf};
use ufop_core::gpg::unique_path;

/// Result for one input file.
#[derive(Debug, Clone, Serialize)]
pub struct ConvertOutcome {
    pub input: String,
    pub output: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConvertCapabilities {
    pub image_formats: Vec<String>,
    pub document_formats: Vec<String>,
    /// LibreOffice was found (document conversion works).
    pub office: bool,
}

#[tauri::command]
pub fn convert_capabilities() -> ConvertCapabilities {
    ConvertCapabilities {
        image_formats: convert_engine::IMAGE_FORMATS.iter().map(|s| s.to_string()).collect(),
        document_formats: convert_engine::DOCUMENT_FORMATS.iter().map(|s| s.to_string()).collect(),
        office: convert_engine::find_office().is_some(),
    }
}

fn target(src: &Path, dest_dir: Option<&str>, name: &str) -> Result<PathBuf, AppError> {
    let dir = match dest_dir.filter(|d| !d.trim().is_empty()) {
        Some(d) => PathBuf::from(d),
        None => src.parent().map(Path::to_path_buf).ok_or_else(|| AppError::validation("Cannot determine the folder"))?,
    };
    std::fs::create_dir_all(&dir).map_err(|e| AppError::file_op(format!("Cannot create {}: {e}", dir.display()), "Check the folder"))?;
    let out = unique_path(&dir.join(name));
    super::policy_commands::enforce_path(&out)?;
    Ok(out)
}

fn outcome(input: String, res: Result<PathBuf, String>) -> ConvertOutcome {
    match res {
        Ok(p) => ConvertOutcome { input, output: Some(p.to_string_lossy().into()), error: None },
        Err(e) => ConvertOutcome { input, output: None, error: Some(e) },
    }
}

/// Resize and/or convert images. Each output is a new file.
#[tauri::command]
pub async fn convert_images(paths: Vec<String>, spec: ImageSpec, dest_dir: Option<String>) -> Result<Vec<ConvertOutcome>, AppError> {
    spec.validate().map_err(AppError::validation)?;
    tokio::task::spawn_blocking(move || {
        paths
            .into_iter()
            .map(|input| {
                let src = PathBuf::from(&input);
                let res = spec
                    .output_name(&src)
                    .and_then(|name| target(&src, dest_dir.as_deref(), &name).map_err(|e| e.user_message()))
                    .and_then(|out| convert_engine::convert_image(&src, &out, &spec).map(|_| out));
                outcome(input, res)
            })
            .collect()
    })
    .await
    .map_err(|e| AppError::internal(format!("Conversion task failed: {e}")))
}

/// Convert office documents with LibreOffice, one at a time.
#[tauri::command]
pub async fn convert_documents(paths: Vec<String>, to: String, dest_dir: Option<String>) -> Result<Vec<ConvertOutcome>, AppError> {
    convert_engine::validate_document_format(&to).map_err(AppError::validation)?;
    let mut out = Vec::with_capacity(paths.len());
    for input in paths {
        let src = PathBuf::from(&input);
        let res = match convert_engine::document_output_name(&src, &to).and_then(|name| target(&src, dest_dir.as_deref(), &name).map_err(|e| e.user_message())) {
            Ok(dest) => convert_engine::convert_document(&src, &dest, &to, std::time::Duration::from_secs(300)).await.map(|_| dest),
            Err(e) => Err(e),
        };
        out.push(outcome(input, res));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn converts_next_to_the_input_without_overwriting() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("pic.png");
        image::RgbaImage::from_pixel(64, 32, image::Rgba([1, 2, 3, 255])).save(&src).unwrap();
        std::fs::write(d.path().join("pic.jpg"), "keep me").unwrap();
        let spec = ImageSpec { format: Some("jpeg".into()), max_width: Some(32), ..Default::default() };
        let s = |p: &Path| p.to_string_lossy().to_string();
        let res = convert_images(vec![s(&src), s(&d.path().join("missing.png"))], spec, None).await.unwrap();
        assert_eq!(res[0].output.as_deref(), Some(s(&d.path().join("pic (2).jpg")).as_str()));
        assert_eq!(std::fs::read_to_string(d.path().join("pic.jpg")).unwrap(), "keep me");
        assert_eq!(image::image_dimensions(d.path().join("pic (2).jpg")).unwrap(), (32, 16));
        assert!(res[1].error.is_some());
        assert!(convert_images(vec![], ImageSpec::default(), None).await.is_err(), "an empty spec is refused");
        assert!(convert_documents(vec![], "exe".into(), None).await.is_err());
    }
}
