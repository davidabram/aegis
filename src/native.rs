use std::fs;
#[cfg(target_os = "macos")]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::transport::bridge::AegisError;

const NATIVE_DIR: &str = "native";
const XCODE_BUILD_DIR: &str = "native/build-xcode";
const XCODE_PROJECT: &str = "native/build-xcode/aegis_native.xcodeproj";
const DEFAULT_SCHEME: &str = "aegis_native";
pub const DEFAULT_APP_BUNDLE_PATH: &str = "native/build-xcode/Debug/aegis_native.app";
#[cfg(target_os = "macos")]
const DEFAULT_BUNDLED_CLI_NAME: &str = "aegis_cli";
const CEF_SDK_DIR: &str =
    "third_party/cef/cef_binary_146.0.6+g68649e2+chromium-146.0.7680.154_macosarm64";

#[derive(Debug, Clone, Serialize)]
pub struct NativeStatus {
    pub cef_sdk_root: PathBuf,
    pub cef_sdk_present: bool,
    pub xcode_project: PathBuf,
    pub xcode_project_present: bool,
    pub default_app_bundle: PathBuf,
    pub default_app_bundle_present: bool,
    pub default_app_executable: PathBuf,
    pub default_app_executable_present: bool,
    pub default_host_library: PathBuf,
    pub default_host_library_present: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum NativeConfiguration {
    Debug,
    Release,
}

impl NativeConfiguration {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "Debug",
            Self::Release => "Release",
        }
    }
}

pub fn status(root: impl AsRef<Path>) -> NativeStatus {
    let root = root.as_ref();
    let cef_sdk_root = root.join(CEF_SDK_DIR);
    let xcode_project = root.join(XCODE_PROJECT);
    let default_app_bundle = root.join(DEFAULT_APP_BUNDLE_PATH);
    let default_app_executable = bundle_executable(&default_app_bundle);
    let default_host_library = root.join("native/build-xcode/Debug/libaegis_host.dylib");

    NativeStatus {
        cef_sdk_present: cef_sdk_root.exists(),
        cef_sdk_root,
        xcode_project_present: xcode_project.exists(),
        xcode_project,
        default_app_bundle_present: default_app_bundle.exists(),
        default_app_bundle,
        default_app_executable_present: default_app_executable.exists(),
        default_app_executable,
        default_host_library_present: default_host_library.exists(),
        default_host_library,
    }
}

pub fn configure_xcode(root: impl AsRef<Path>) -> Result<PathBuf, AegisError> {
    let root = root.as_ref();
    let native_dir = root.join(NATIVE_DIR);
    let build_dir = root.join(XCODE_BUILD_DIR);

    run_checked(
        "cmake",
        &[
            "-S",
            native_dir.to_str().ok_or_else(path_encoding_error)?,
            "-B",
            build_dir.to_str().ok_or_else(path_encoding_error)?,
            "-G",
            "Xcode",
            &format!("-DPROJECT_ARCH={}", apple_arch()),
        ],
        root,
    )?;

    Ok(root.join(XCODE_PROJECT))
}

pub fn build_xcode(
    root: impl AsRef<Path>,
    configuration: NativeConfiguration,
    scheme: Option<&str>,
) -> Result<PathBuf, AegisError> {
    let root = root.as_ref();
    let project = root.join(XCODE_PROJECT);
    if !project.exists() {
        configure_xcode(root)?;
    }
    let scheme = scheme.unwrap_or(DEFAULT_SCHEME);

    run_checked(
        "xcodebuild",
        &[
            "-project",
            project.to_str().ok_or_else(path_encoding_error)?,
            "-scheme",
            scheme,
            "-configuration",
            configuration.as_str(),
            "-arch",
            apple_arch(),
            "CODE_SIGNING_ALLOWED=NO",
            "CODE_SIGNING_REQUIRED=NO",
            "CODE_SIGN_IDENTITY=",
            "build",
        ],
        root,
    )?;

    Ok(artifact_for_scheme(root, configuration, scheme))
}

pub fn bundle_executable(bundle: impl AsRef<Path>) -> PathBuf {
    let bundle = bundle.as_ref();
    let binary_name = bundle
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| DEFAULT_SCHEME.to_string());
    bundle.join("Contents").join("MacOS").join(binary_name)
}

pub fn artifact_for_scheme(
    root: impl AsRef<Path>,
    configuration: NativeConfiguration,
    scheme: &str,
) -> PathBuf {
    let base = root
        .as_ref()
        .join("native/build-xcode")
        .join(configuration.as_str());
    match scheme {
        "aegis_host" => base.join("libaegis_host.dylib"),
        _ => bundle_executable(base.join("aegis_native.app")),
    }
}

#[cfg(target_os = "macos")]
pub fn is_bundle_executable(path: &Path) -> bool {
    let Some(macos_dir) = path.parent() else {
        return false;
    };
    if macos_dir.file_name().and_then(|name| name.to_str()) != Some("MacOS") {
        return false;
    }

    let Some(contents_dir) = macos_dir.parent() else {
        return false;
    };
    if contents_dir.file_name().and_then(|name| name.to_str()) != Some("Contents") {
        return false;
    }

    contents_dir
        .parent()
        .and_then(|app| app.extension())
        .and_then(|ext| ext.to_str())
        == Some("app")
}

#[cfg(target_os = "macos")]
pub fn prepare_bundled_cli(
    root: impl AsRef<Path>,
    source_executable: impl AsRef<Path>,
) -> Result<PathBuf, AegisError> {
    let root = root.as_ref();
    let app_bundle = root.join(DEFAULT_APP_BUNDLE_PATH);
    if !app_bundle.exists() {
        build_xcode(root, NativeConfiguration::Debug, None)?;
    }

    let target = app_bundle
        .join("Contents")
        .join("MacOS")
        .join(DEFAULT_BUNDLED_CLI_NAME);
    let source = source_executable.as_ref();

    if target.exists() {
        let same_file = fs::canonicalize(&target).ok() == fs::canonicalize(source).ok();
        if !same_file {
            fs::remove_file(&target)?;
        }
    }

    if !target.exists() {
        fs::copy(source, &target)?;
        let mode = fs::metadata(source)?.permissions().mode();
        fs::set_permissions(&target, fs::Permissions::from_mode(mode))?;
    }

    Ok(target)
}

fn run_checked(program: &str, args: &[&str], root: &Path) -> Result<(), AegisError> {
    let output = Command::new(program)
        .args(args)
        .current_dir(root)
        .output()?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(AegisError::Bridge(format!(
        "{program} failed with status {}: {}{}{}",
        output
            .status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_string()),
        stdout.trim(),
        if !stdout.trim().is_empty() && !stderr.trim().is_empty() {
            " | "
        } else {
            ""
        },
        stderr.trim()
    )))
}

fn apple_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => other,
    }
}

fn path_encoding_error() -> AegisError {
    AegisError::Bridge("path is not valid utf-8".into())
}

#[cfg(test)]
mod tests {
    use super::is_bundle_executable;
    use std::path::Path;

    #[test]
    fn detects_bundle_executable_paths() {
        assert!(is_bundle_executable(Path::new(
            "/tmp/aegis_native.app/Contents/MacOS/aegis_cli"
        )));
        assert!(!is_bundle_executable(Path::new(
            "/tmp/aegis_native/Contents/MacOS/aegis_cli"
        )));
        assert!(!is_bundle_executable(Path::new("/tmp/aegis_cli")));
    }
}
