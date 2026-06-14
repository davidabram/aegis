use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::state::AegisStatePaths;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTransferPaths {
    pub download_dir: PathBuf,
    pub upload_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedUploadFile {
    pub source_path: PathBuf,
    pub staged_path: PathBuf,
}

pub fn resolve_transfer_paths(
    download_dir: Option<PathBuf>,
    upload_dir: Option<PathBuf>,
) -> Result<ResolvedTransferPaths, String> {
    let state = AegisStatePaths::detect()?;
    let download_dir = ensure_transfer_dir(
        download_dir.unwrap_or_else(|| state.downloads_dir()),
        "download",
    )?;
    let upload_dir =
        ensure_transfer_dir(upload_dir.unwrap_or_else(|| state.uploads_dir()), "upload")?;
    Ok(ResolvedTransferPaths {
        download_dir,
        upload_dir,
    })
}

pub fn ensure_transfer_dir(path: PathBuf, label: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(&path).map_err(|error| {
        format!(
            "failed to create {label} directory {}: {error}",
            path.display()
        )
    })?;
    Ok(path)
}

pub fn stage_upload_file(source: &Path, upload_dir: &Path) -> Result<StagedUploadFile, String> {
    let metadata = fs::metadata(source)
        .map_err(|error| format!("failed to stat upload file {}: {error}", source.display()))?;
    if !metadata.is_file() {
        return Err(format!("upload path {} is not a file", source.display()));
    }
    fs::create_dir_all(upload_dir).map_err(|error| {
        format!(
            "failed to create upload staging directory {}: {error}",
            upload_dir.display()
        )
    })?;

    let staged_name = unique_staged_name(source);
    let staged_path = upload_dir.join(staged_name);
    fs::copy(source, &staged_path).map_err(|error| {
        format!(
            "failed to stage upload file {} into {}: {error}",
            source.display(),
            staged_path.display()
        )
    })?;

    Ok(StagedUploadFile {
        source_path: source.to_path_buf(),
        staged_path,
    })
}

fn unique_staged_name(source: &Path) -> String {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let file_name = source
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("upload.bin");
    format!("{stamp}-{}", sanitize_component(file_name))
}

fn sanitize_component(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => ch,
            _ => '_',
        })
        .collect::<String>();
    let trimmed = sanitized.trim_matches('_');
    if trimmed.is_empty() {
        "file".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::aegis_test_env_lock;

    #[test]
    fn resolves_default_transfer_paths_inside_aegis_home() {
        let _guard = aegis_test_env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let temp = tempfile::tempdir().expect("tempdir should exist");
        unsafe {
            std::env::set_var("AEGIS_HOME", temp.path());
        }
        let paths =
            resolve_transfer_paths(None, None).expect("default transfer paths should resolve");
        assert_eq!(paths.download_dir, temp.path().join("files/downloads"));
        assert_eq!(paths.upload_dir, temp.path().join("files/uploads"));
        assert!(paths.download_dir.exists());
        assert!(paths.upload_dir.exists());
        unsafe {
            std::env::remove_var("AEGIS_HOME");
        }
    }

    #[test]
    fn stages_upload_files_in_target_directory() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        let source = temp.path().join("my file?.pdf");
        let target_dir = temp.path().join("uploads");
        fs::write(&source, b"pdf").expect("fixture should write");

        let staged = stage_upload_file(&source, &target_dir).expect("upload should stage");
        assert!(staged.staged_path.exists());
        assert_eq!(
            fs::read(&staged.staged_path).expect("staged contents should read"),
            b"pdf"
        );
        assert!(
            staged
                .staged_path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.ends_with("my_file_.pdf"))
        );
    }
}
