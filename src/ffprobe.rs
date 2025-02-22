//! Utilities related to the FFprobe binary.

use crate::command::BackgroundCommand;
use anyhow::Context;
use std::{env::current_exe, ffi::OsStr, path::PathBuf};
use std::{path::Path, process::Stdio};

use tokio::process::Command;

/// Returns the path of the downloaded FFprobe executable, or falls back to
/// assuming its installed in the system path. Note that not all FFmpeg
/// distributions include FFprobe.
pub fn ffprobe_path() -> PathBuf {
  let default = Path::new("ffprobe").to_path_buf();
  match ffprobe_sidecar_path() {
    Ok(sidecar_path) => match sidecar_path.exists() {
      true => sidecar_path,
      false => default,
    },
    Err(_) => default,
  }
}

/// The (expected) path to an FFmpeg binary adjacent to the Rust binary.
///
/// The extension between platforms, with Windows using `.exe`, while Mac and
/// Linux have no extension.
pub fn ffprobe_sidecar_path() -> anyhow::Result<PathBuf> {
  let mut path = current_exe()?
    .parent()
    .context("Can't get parent of current_exe")?
    .join("ffmpeg_dir")
    .join("ffprobe");
  if cfg!(windows) {
    path.set_extension("exe");
  }
  Ok(path)
}

/// Alias for `ffprobe -version`, parsing the version number and returning it.
pub async fn ffprobe_version() -> anyhow::Result<String> {
  ffprobe_version_with_path(ffprobe_path()).await
}

/// Lower level variant of `ffprobe_version` that exposes a customized the path
/// to the ffmpeg binary.
pub async fn ffprobe_version_with_path<S: AsRef<OsStr>>(path: S) -> anyhow::Result<String> {
  let output = Command::new(&path)
    .arg("-version")
    .create_no_window()
    .output()
    .await?;

  // note:version parsing is not implemented for ffprobe

  Ok(String::from_utf8(output.stdout)?)
}

/// Verify whether ffprobe is installed on the system. This will return true if
/// there is a ffprobe binary in the PATH, or in the same directory as the Rust
/// executable.
pub async fn ffprobe_is_installed() -> bool {
  Command::new(ffprobe_path())
    .create_no_window()
    .arg("-version")
    .stderr(Stdio::null())
    .stdout(Stdio::null())
    .status()
    .await
    .map(|s| s.success())
    .unwrap_or_else(|_| false)
}
