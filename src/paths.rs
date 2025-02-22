//! Utilities for locating FFmpeg binaries on the system

use anyhow::Context;
use std::{
  env::current_exe,
  path::{Path, PathBuf},
};

/// Returns the default path of the FFmpeg executable, to be used as the
/// argument to `Command::new`. It should first attempt to locate an FFmpeg
/// binary adjacent to the Rust executable. If that fails, it should invoke
/// `ffmpeg` expecting it to be in the system path. If that fails, an
/// informative error message should be printed (not when this function is
/// called, but when the command is actually run).
pub fn ffmpeg_path() -> PathBuf {
  let default = Path::new("ffmpeg").to_path_buf();
  match sidecar_path() {
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
pub fn sidecar_path() -> anyhow::Result<PathBuf> {
  let mut path = current_exe()?
    .parent()
    .context("Can't get parent of current_exe")?
    .join("ffmpeg_dir")
    .join("ffmpeg");
  if cfg!(windows) {
    path.set_extension("exe");
  }
  Ok(path)
}

/// By default, downloads all temporary files to the same directory as the Rust executable.
pub fn sidecar_dir() -> anyhow::Result<PathBuf> {
  Ok(
    sidecar_path()?
      .parent()
      .context("invalid sidecar path")?
      .to_path_buf(),
  )
}
