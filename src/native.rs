use std::env;
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
pub const DEFAULT_APP_BUNDLE_PATH: &str = "native/build-xcode/Release/aegis_native.app";
#[cfg(target_os = "macos")]
const DEFAULT_BUNDLED_CLI_NAME: &str = "aegis_cli";
#[cfg(target_os = "macos")]
const LOCAL_INSTALL_APP_NAME: &str = "Aegis.app";
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
    let default_app_bundle = preferred_app_bundle(root);
    let default_app_executable = bundle_executable(&default_app_bundle);
    let default_host_library = root.join("native/build-xcode/Release/libaegis_host.dylib");

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

#[cfg(target_os = "macos")]
pub fn install_local_release(
    root: impl AsRef<Path>,
    source_executable: impl AsRef<Path>,
) -> Result<PathBuf, AegisError> {
    let root = root.as_ref();
    let source_executable = source_executable.as_ref();
    let build_output_bundle = root.join(DEFAULT_APP_BUNDLE_PATH);
    let install_bundle = installed_app_bundle()
        .ok_or_else(|| AegisError::Bridge("HOME is not set; cannot resolve local app install path".into()))?;

    build_xcode(root, NativeConfiguration::Release, Some("aegis_host"))?;
    build_xcode(root, NativeConfiguration::Release, None)?;

    if install_bundle.exists() {
        fs::remove_dir_all(&install_bundle)?;
    }
    if let Some(parent) = install_bundle.parent() {
        fs::create_dir_all(parent)?;
    }
    copy_dir_recursive(&build_output_bundle, &install_bundle)?;

    let bundled_cli = install_bundle
        .join("Contents")
        .join("MacOS")
        .join(DEFAULT_BUNDLED_CLI_NAME);
    fs::copy(source_executable, &bundled_cli)?;
    let mode = fs::metadata(source_executable)?.permissions().mode();
    fs::set_permissions(&bundled_cli, fs::Permissions::from_mode(mode))?;

    clear_quarantine_attribute(&install_bundle);
    ad_hoc_sign_bundle(&install_bundle)?;

    Ok(install_bundle)
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
            "build",
        ],
        root,
    )?;

    Ok(artifact_for_scheme(root, configuration, scheme))
}

pub fn bundle_executable(bundle: impl AsRef<Path>) -> PathBuf {
    let bundle = bundle.as_ref();
    let binary_name = macos_bundle_executable_name(bundle).unwrap_or_else(|| {
        bundle
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| DEFAULT_SCHEME.to_string())
    });
    bundle.join("Contents").join("MacOS").join(binary_name)
}

#[cfg(target_os = "macos")]
fn macos_bundle_executable_name(bundle: &Path) -> Option<String> {
    let plist = bundle.join("Contents").join("Info");
    let output = Command::new("defaults")
        .arg("read")
        .arg(plist)
        .arg("CFBundleExecutable")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(not(target_os = "macos"))]
fn macos_bundle_executable_name(_bundle: &Path) -> Option<String> {
    None
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
fn preferred_app_bundle(root: &Path) -> PathBuf {
    installed_app_bundle().unwrap_or_else(|| root.join(DEFAULT_APP_BUNDLE_PATH))
}

#[cfg(not(target_os = "macos"))]
fn preferred_app_bundle(root: &Path) -> PathBuf {
    root.join(DEFAULT_APP_BUNDLE_PATH)
}

#[cfg(target_os = "macos")]
fn installed_app_bundle() -> Option<PathBuf> {
    let home = env::var_os("HOME")?;
    let bundle = PathBuf::from(home)
        .join("Applications")
        .join(LOCAL_INSTALL_APP_NAME);
    bundle.exists().then_some(bundle)
}

#[cfg(target_os = "macos")]
fn clear_quarantine_attribute(bundle: &Path) {
    let _ = Command::new("xattr").args(["-cr"]).arg(bundle).output();
}

#[cfg(target_os = "macos")]
fn ad_hoc_sign_bundle(bundle: &Path) -> Result<(), AegisError> {
    run_checked(
        "codesign",
        &[
            "--force",
            "--deep",
            "--sign",
            "-",
            bundle.to_str().ok_or_else(path_encoding_error)?,
        ],
        Path::new("/"),
    )
}

#[cfg(target_os = "macos")]
fn copy_dir_recursive(source: &Path, target: &Path) -> Result<(), AegisError> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &target_path)?;
        } else if file_type.is_symlink() {
            let link_target = fs::read_link(&source_path)?;
            std::os::unix::fs::symlink(&link_target, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path)?;
        }
    }
    Ok(())
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
