use crate::command::BackgroundCommand;
use crate::event::FfmpegEvent;
use crate::log_parser::FfmpegLogParser;
use crate::paths::ffmpeg_path;
use anyhow::Context;
use std::ffi::OsStr;
use std::process::Stdio;
use tokio::io::BufReader;
use tokio::process::Command;

/// Alias for `ffmmpeg -version`, parsing the version number and returning it.
pub async fn ffmpeg_version() -> anyhow::Result<String> {
  ffmpeg_version_with_path(ffmpeg_path()).await
}

/// Lower level variant of `ffmpeg_version`  that exposes a customized path
/// to the ffmepg binary
pub async fn ffmpeg_version_with_path<P: AsRef<OsStr>>(path: P) -> anyhow::Result<String> {
  let mut cmd = Command::new(&path)
    .create_no_window()
    .arg("-version")
    .stdout(Stdio::piped())
    .spawn()?;

  let stdout = cmd.stdout.take().context("no stdout channel")?;
  let reader = BufReader::new(stdout);
  let mut parser = FfmpegLogParser::new(reader);

  let mut version: Option<String> = None;
  while let Ok(event) = parser.parse_next_event().await {
    match event {
      FfmpegEvent::ParsedVersion(v) => version = Some(v.version),
      FfmpegEvent::LogEOF => break,
      _ => {}
    }
  }

  let exit_status = cmd.wait().await?;
  if !exit_status.success() {
    anyhow::bail!("ffmpeg -version exited with non-zero status");
  }

  version.context("failed to parse ffmpeg version")
}
