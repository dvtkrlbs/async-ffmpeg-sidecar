//! Utilities for downloading and unpacking FFmpeg binaries

use anyhow::Result;

#[cfg(feature = "download_ffmpeg")]
use std::path::{Path, PathBuf};
#[cfg(feature = "download_ffmpeg")]
use tokio::fs::File;

/// The default directory name for unpacking a downloaded FFmpeg release archive.
pub const UNPACK_DIRNAME: &str = "ffmpeg_release_temp";

/// URL of a manifest file containing the latest published build of FFmpeg. The
/// correct URL for the target platform is baked in at compile time.
pub fn ffmpeg_manifest_url() -> Result<&'static str> {
  if cfg!(not(target_arch = "x86_64")) {
    anyhow::bail!("Downloads must be manually provided for non-x86_64 architectures");
  }

  if cfg!(target_os = "windows") {
    Ok("https://www.gyan.dev/ffmpeg/builds/release-version")
  } else if cfg!(target_os = "macos") {
    Ok("https://evermeet.cx/ffmpeg/info/ffmpeg/release")
  } else if cfg!(target_os = "linux") {
    Ok("https://johnvansickle.com/ffmpeg/release-readme.txt")
  } else {
    anyhow::bail!("Unsupported platform")
  }
}

/// URL for the latest published FFmpeg release. The correct URL for the target
/// platform is baked in at compile time.
pub fn ffmpeg_download_url() -> Result<&'static str> {
  if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
    Ok("https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip")
  } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    Ok("https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-amd64-static.tar.xz")
  } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
    Ok("https://evermeet.cx/ffmpeg/getrelease/zip")
  } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    Ok("https://www.osxexperts.net/ffmpeg7arm.zip") // Mac M1
  } else {
    anyhow::bail!("Unsupported platform; you can provide your own URL instead and call download_ffmpeg_package directly.")
  }
}

/// Check if FFmpeg is installed, and if it's not, download and unpack it.
/// Automatically selects the correct binaries for Windows, Linux, and MacOS.
/// The binaries will be placed in the same directory as the Rust executable.
///
/// If FFmpeg is already installed, the method exits early without downloading
/// anything.
#[cfg(feature = "download_ffmpeg")]
pub async fn auto_download() -> Result<()> {
  use crate::{command::ffmpeg_is_installed, paths::sidecar_dir};

  if ffmpeg_is_installed().await {
    return Ok(());
  }

  let download_url = ffmpeg_download_url()?;
  let destination = sidecar_dir()?;
  tokio::fs::create_dir_all(&destination).await?;
  let archive_path = download_ffmpeg_package(download_url, &destination).await?;
  unpack_ffmpeg(&archive_path, &destination).await?;

  if !(ffmpeg_is_installed().await) {
    anyhow::bail!("Ffmpeg failed to install, please install manually")
  }

  Ok(())
}

/// Parse the macOS version number from a JSON string manifest file.
///
/// Example input: <https://evermeet.cx/ffmpeg/info/ffmpeg/release>
///
/// ```rust
/// use async_ffmpeg_sidecar::download::parse_macos_version;
/// let json_string = "{\"name\":\"ffmpeg\",\"type\":\"release\",\"version\":\"6.0\",...}";
/// let parsed = parse_macos_version(&json_string).unwrap();
/// assert_eq!(parsed, "6.0");
/// ```
pub fn parse_macos_version(version: &str) -> Option<String> {
  version
    .split("\"version\":")
    .nth(1)?
    .trim()
    .split('\"')
    .nth(1)
    .map(|s| s.to_string())
}

/// Parse the Linux version number from a long manifest text file.
///
/// Example input: <https://johnvansickle.com/ffmpeg/release-readme.txt>
///
/// ```rust
/// use async_ffmpeg_sidecar::download::parse_linux_version;
/// let json_string = "build: ffmpeg-5.1.1-amd64-static.tar.xz\nversion: 5.1.1\n\ngcc: 8.3.0";
/// let parsed = parse_linux_version(&json_string).unwrap();
/// assert_eq!(parsed, "5.1.1");
/// ```
pub fn parse_linux_version(version: &str) -> Option<String> {
  version
    .split("version:")
    .nth(1)?
    .split_whitespace()
    .next()
    .map(|s| s.to_string())
}

/// Makes an HTTP request to obtain the latest version available online,
/// automatically choosing the correct URL for the current platform.
#[cfg(feature = "download_ffmpeg")]
pub async fn check_latest_version() -> Result<String> {
  use anyhow::Context;

  // Mac M1 doesn't have a manifest URL, so match version provided
  if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    return Ok("7.0".to_string());
  }

  let manifest_url = ffmpeg_manifest_url()?;
  let version_string = reqwest::get(manifest_url)
    .await?
    .error_for_status()?
    .text()
    .await?;

  if cfg!(target_os = "windows") {
    Ok(version_string)
  } else if cfg!(target_os = "macos") {
    parse_macos_version(&version_string).context("failed to parse version number (macos variant)")
  } else if cfg!(target_os = "linux") {
    parse_linux_version(&version_string).context("failed to parse version number (linux variant)")
  } else {
    anyhow::bail!("unsupported platform")
  }
}

/// Make an HTTP request to download an archive from the latest published release online
#[cfg(feature = "download_ffmpeg")]
pub async fn download_ffmpeg_package(url: &str, download_dir: &Path) -> Result<PathBuf> {
  use anyhow::Context;
  use tokio::fs::File;
  use tokio::io::AsyncWriteExt;
  use futures_util::StreamExt;

  let filename = Path::new(url)
    .file_name()
    .context("Failed to get filename")?;

  let archive_path = download_dir.join(filename);

  let response = reqwest::get(url)
    .await
    .context("failed to download ffmpeg")?
    .error_for_status()
    .context("server returned error")?;

  let mut file = File::create(&archive_path)
    .await
    .context("failed to create file for ffmpeg download")?;

  let mut stream = response.bytes_stream();

  while let Some(chunk) = stream.next().await {
    let data = chunk?;
    file.write_all(&data).await?
  }

  Ok(archive_path)
}

/// After downloading unpacks the archive to a folder, moves the binaries to
/// their final location, and deletes the archive and temporary folder.
#[cfg(feature = "download_ffmpeg")]
pub async fn unpack_ffmpeg(from_archive: &PathBuf, binary_folder: &Path) -> Result<()> {
  use anyhow::Context;
  use tokio::fs::{create_dir_all, read_dir, remove_dir_all, remove_file, File};

  let temp_folder = binary_folder.join(UNPACK_DIRNAME);
  create_dir_all(&temp_folder)
    .await
    .context("failed creating temp dir")?;

  let file = File::open(from_archive)
    .await
    .context("failed to open archive")?;

  #[cfg(target_os = "linux")]
  {
    untar_file(file, &temp_folder).await?
  }

  #[cfg(not(target_os = "linux"))]
  {
    unzip_file(file, &temp_folder).await?
  }

  let inner_folder = read_dir(&temp_folder)
    .await?
    .next_entry()
    .await
    .context("Failed to get inner folder")?
    .unwrap();
  let (ffmpeg, ffplay, ffprobe) = if cfg!(target_os = "windows") {
    (
      inner_folder.path().join("bin/ffmpeg.exe"),
      inner_folder.path().join("bin/ffplay.exe"),
      inner_folder.path().join("bin/ffprobe.exe"),
    )
  } else if cfg!(target_os = "linux") {
    (
      inner_folder.path().join("./ffmpeg"),
      inner_folder.path().join("./ffplay"), // <- no ffplay on linux
      inner_folder.path().join("./ffprobe"),
    )
  } else if cfg!(target_os = "macos") {
    (
      temp_folder.join("ffmpeg"),
      temp_folder.join("ffplay"),  // <- no ffplay on mac
      temp_folder.join("ffprobe"), // <- no ffprobe on mac
    )
  } else {
    anyhow::bail!("Unsupported platform");
  };

  set_executable_permission(&ffmpeg).await?;
  move_bin(&ffmpeg, binary_folder).await?;

  if ffprobe.exists() {
    set_executable_permission(&ffprobe).await?;
    move_bin(&ffprobe, binary_folder).await?;
  }

  if ffplay.exists() {
    set_executable_permission(&ffplay).await?;
    move_bin(&ffplay, binary_folder).await?;
  }

  // Delete archive and unpacked files
  if temp_folder.exists() && temp_folder.is_dir() {
    remove_dir_all(&temp_folder).await?;
  }

  if from_archive.exists() {
    remove_file(from_archive).await?;
  }

  Ok(())
}

#[cfg(feature = "download_ffmpeg")]
async fn move_bin(path: &Path, binary_folder: &Path) -> Result<()> {
  use tokio::fs::rename;
  use anyhow::Context;
  let file_name = binary_folder.join(
    path
      .file_name()
      .with_context(|| format!("Path {} does not have a file_name", path.to_string_lossy()))?,
  );

  rename(path, file_name).await?;
  anyhow::Ok(())
}
#[cfg(all(feature = "download_ffmpeg", target_family = "unix"))]
async fn set_executable_permission(path: &Path) -> Result<()> {
  #[cfg(target_family = "unix")]
  {
    use tokio::fs::set_permissions;

    use std::os::unix::fs::PermissionsExt;
    let mut perms = path.metadata()?.permissions();

    perms.set_mode(perms.mode() | 0o100);

    set_permissions(path, perms).await?;
  }

  Ok(())
}

#[cfg(all(feature = "download_ffmpeg", not(target_family = "unix")))]
async fn set_executable_permission(_path: &Path) -> Result<()> {
  Ok(())
}

#[cfg(all(feature = "download_ffmpeg", not(target_os = "linux")))]
async fn unzip_file(archive: File, out_dir: &Path) -> Result<()> {
  use async_zip::base::read::seek::ZipFileReader;
  use tokio::fs::create_dir_all;
  use tokio::fs::OpenOptions;
  use tokio::io::BufReader;
  use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
  use anyhow::Context;

  let archive = BufReader::new(archive).compat();

  let mut reader = ZipFileReader::new(archive)
    .await
    .context("Failed to read zip file")?;

  for index in 0..reader.file().entries().len() {
    let entry = reader.file().entries().get(index).unwrap();
    let path = out_dir.join(sanitize_file_path(entry.filename().as_str()?));
    // If the filename of the entry ends with '/', it is treated as a directory.
    // This is implemented by previous versions of this crate and the Python Standard Library.
    // https://docs.rs/async_zip/0.0.8/src/async_zip/read/mod.rs.html#63-65
    // https://github.com/python/cpython/blob/820ef62833bd2d84a141adedd9a05998595d6b6d/Lib/zipfile.py#L528
    let entry_is_dir = entry.dir()?;

    let mut entry_reader = reader
      .reader_without_entry(index)
      .await
      .expect("Failed to read ZipEntry");

    if entry_is_dir {
      // The directory may have been created if iteration is out of order.
      if !path.exists() {
        create_dir_all(&path)
          .await
          .expect("Failed to create extracted directory");
      }
    } else {
      // Creates parent directories. They may not exist if iteration is out of order
      // or the archive does not contain directory entries.
      let parent = path
        .parent()
        .expect("A file entry should have parent directories");
      if !parent.is_dir() {
        create_dir_all(parent)
          .await
          .expect("Failed to create parent directories");
      }
      let writer = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .await
        .expect("Failed to create extracted file");
      futures_util::io::copy(&mut entry_reader, &mut writer.compat_write())
        .await
        .expect("Failed to copy to extracted file");

      // Closes the file and manipulates its metadata here if you wish to preserve its metadata from the archive.
    }
  }

  Ok(())
}

/// Returns a relative path without reserved names, redundant separators, ".", or "..".
#[cfg(all(feature = "download_ffmpeg", not(target_os = "linux")))]
fn sanitize_file_path(path: &str) -> PathBuf {
  // Replaces backwards slashes
  path
    .replace('\\', "/")
    // Sanitizes each component
    .split('/')
    .map(sanitize_filename::sanitize)
    .collect()
}

#[cfg(all(feature = "download_ffmpeg", target_os = "linux"))]
async fn untar_file(archive: File, out_dir: &Path) -> Result<()> {
  use async_compression::tokio::bufread::XzDecoder;
  use tokio::io::BufReader;
  use tokio_tar::Archive;
  let archive = BufReader::new(archive);
  let archive = XzDecoder::new(archive);
  let mut archive = Archive::new(archive);

  archive.unpack(out_dir).await?;

  Ok(())
}
