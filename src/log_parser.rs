// mostly copied from https://github.com/nathanbabcock/ffmpeg-sidecar/blob/ba61acabecbf360c7f0d0228233641396012878c/src/log_parser.rs
// and adapted to use tokio instead sync

use crate::comma_iter::CommaIter;
use crate::event::{
  AudioStream, FfmpegConfiguration, FfmpegDuration, FfmpegEvent, FfmpegInput, FfmpegOutput,
  FfmpegProgress, FfmpegStream, FfmpegVersion, LogLevel, StreamTypeSpecificData, VideoStream,
};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader, Lines};

#[derive(Debug, Clone, PartialEq)]
enum LogSection {
  Input(u32),
  Output(u32),
  StreamMapping,
  Other,
}

pub struct FfmpegLogParser<R: AsyncBufRead + Unpin> {
  lines: Lines<BufReader<R>>,
  cur_section: LogSection,
}

impl<R: AsyncBufRead + Unpin> FfmpegLogParser<R> {
  /// Consume lines from the inner reader until obtaining a completed
  /// `FfmpegEvent`, returning it.
  ///
  /// Typically, this consumes a single line, but in the case of multi-line
  /// input/output stream specifications, nested method calls will consume
  /// additional lines until the entire vector of Input/Outputs is parsed.
  ///
  /// Line endings can be marked by three possible delimiters:
  /// - `\n` (macOS)
  /// - `\r\n` (Windows)
  /// - `\r` (Windows, progress updates which overwrite the previous line)
  pub async fn parse_next_event(&mut self) -> anyhow::Result<FfmpegEvent> {
    let Some(line) = self.lines.next_line().await? else {
      return Ok(FfmpegEvent::LogEOF);
    };

    // let line = from_utf8(line)?.trim();
    let raw_log_message = line.clone();

    // Track log section
    if let Some(input_number) = try_parse_input(&line) {
      self.cur_section = LogSection::Input(input_number);
      return Ok(FfmpegEvent::ParsedInput(FfmpegInput {
        index: input_number,
        duration: None,
        raw_log_message,
      }));
    } else if let Some(output) = try_parse_output(&line) {
      self.cur_section = LogSection::Output(output.index);
      return Ok(FfmpegEvent::ParsedOutput(output));
    } else if line.contains("Stream mapping:") {
      self.cur_section = LogSection::StreamMapping;
    }

    // Parse
    if let Some(version) = try_parse_version(&line) {
      Ok(FfmpegEvent::ParsedVersion(FfmpegVersion {
        version,
        raw_log_message,
      }))
    } else if let Some(configuration) = try_parse_configuration(&line) {
      Ok(FfmpegEvent::ParsedConfiguration(FfmpegConfiguration {
        configuration,
        raw_log_message,
      }))
    } else if let Some(duration) = try_parse_duration(&line) {
      match self.cur_section {
        LogSection::Input(input_index) => Ok(FfmpegEvent::ParsedDuration(FfmpegDuration {
          input_index,
          duration,
          raw_log_message,
        })),
        _ => Ok(FfmpegEvent::Log(LogLevel::Info, line.to_string())),
      }
    } else if self.cur_section == LogSection::StreamMapping && line.contains("  Stream #") {
      Ok(FfmpegEvent::ParsedStreamMapping(line.to_string()))
    } else if let Some(stream) = try_parse_stream(&line) {
      match self.cur_section {
        LogSection::Input(_) => Ok(FfmpegEvent::ParsedInputStream(stream)),
        LogSection::Output(_) => Ok(FfmpegEvent::ParsedOutputStream(stream)),
        LogSection::Other | LogSection::StreamMapping => Err(anyhow::Error::msg(format!(
          "Unexpected stream specification: {}",
          line
        ))),
      }
    } else if let Some(progress) = try_parse_progress(&line) {
      self.cur_section = LogSection::Other;
      Ok(FfmpegEvent::Progress(progress))
    } else if line.contains("[info]") {
      Ok(FfmpegEvent::Log(LogLevel::Info, line.to_string()))
    } else if line.contains("[warning]") {
      Ok(FfmpegEvent::Log(LogLevel::Warning, line.to_string()))
    } else if line.contains("[error]") {
      Ok(FfmpegEvent::Log(LogLevel::Error, line.to_string()))
    } else if line.contains("[fatal]") {
      Ok(FfmpegEvent::Log(LogLevel::Fatal, line.to_string()))
    } else {
      Ok(FfmpegEvent::Log(LogLevel::Unknown, line.to_string()))
    }
  }

  pub fn new(inner: R) -> Self {
    let buf_read = BufReader::new(inner);
    let lines = buf_read.lines();

    Self {
      lines,
      cur_section: LogSection::Other,
    }
  }
}

/// Parses the ffmpeg version string from the stderr stream,
/// typically the very first line of output:
///
/// ```rust
/// use async_ffmpeg_sidecar::log_parser::try_parse_version;
///
/// let line = "[info] ffmpeg version 2023-01-18-git-ba36e6ed52-full_build-www.gyan.dev Copyright (c) 2000-2023 the FFmpeg developers\n";
///
/// let version = try_parse_version(line).unwrap();
///
/// assert_eq!(version, "2023-01-18-git-ba36e6ed52-full_build-www.gyan.dev");
/// ```
pub fn try_parse_version(string: &str) -> Option<String> {
  string
    .strip_prefix("[info]")
    .unwrap_or(string)
    .trim()
    .strip_prefix("ffmpeg version ")?
    .split_whitespace()
    .next()
    .map(|s| s.to_string())
}

/// Parses the list of configuration flags ffmpeg was built with.
/// Typically the second line of log output.
///
/// ## Example:
///
/// ```rust
/// use async_ffmpeg_sidecar::log_parser::try_parse_configuration;
///
/// let line = "[info]   configuration: --enable-gpl --enable-version3 --enable-static\n";
/// // Typically much longer, 20-30+ flags
///
/// let version = try_parse_configuration(line).unwrap();
///
/// assert_eq!(version.len(), 3);
/// assert_eq!(version[0], "--enable-gpl");
/// assert_eq!(version[1], "--enable-version3");
/// assert_eq!(version[2], "--enable-static");
/// ```
///
pub fn try_parse_configuration(string: &str) -> Option<Vec<String>> {
  string
    .strip_prefix("[info]")
    .unwrap_or(string)
    .trim()
    .strip_prefix("configuration: ")
    .map(|s| s.split_whitespace().map(|s| s.to_string()).collect())
}

/// Parse an input section like the following extracting the index of the input:
///
/// ## Example
///
/// ```rust
/// use async_ffmpeg_sidecar::log_parser::try_parse_input;
/// let line = "[info] Input #0, lavfi, from 'testsrc=duration=5':\n";
/// let input = try_parse_input(line);
/// assert_eq!(input, Some(0));
/// ```
pub fn try_parse_input(string: &str) -> Option<u32> {
  string
    .strip_prefix("[info]")
    .unwrap_or(string)
    .trim()
    .strip_prefix("Input #")?
    .split_whitespace()
    .next()
    .and_then(|s| s.split(',').next())
    .and_then(|s| s.parse::<u32>().ok())
}

/// ## Example
///
/// ```rust
/// use async_ffmpeg_sidecar::log_parser::try_parse_duration;
/// let line = "[info]   Duration: 00:00:05.00, start: 0.000000, bitrate: 16 kb/s, start: 0.000000, bitrate: N/A\n";
/// let duration = try_parse_duration(line);
/// assert_eq!(duration, Some(5.0));
/// ```
///
/// ### Unknown duration
///
/// ```rust
/// use async_ffmpeg_sidecar::log_parser::try_parse_duration;
/// let line = "[info]   Duration: N/A, start: 0.000000, bitrate: N/A\n";
/// let duration = try_parse_duration(line);
/// assert_eq!(duration, None);
/// ```
pub fn try_parse_duration(string: &str) -> Option<f64> {
  string
    .strip_prefix("[info]")
    .unwrap_or(string)
    .trim()
    .strip_prefix("Duration:")?
    .trim()
    .split(',')
    .next()
    .and_then(parse_time_str)
}

/// Parse an output section like the following, extracting the index of the input
///
/// ## Example:
///
/// ```rust
/// use async_ffmpeg_sidecar::log_parser::try_parse_output;
/// use async_ffmpeg_sidecar::event::FfmpegOutput;
/// let line = "[info] Output #0, mp4, to 'test.mp4':\n";
/// let output = try_parse_output(line);
/// assert_eq!(output, Some(FfmpegOutput {
///     index: 0,
///     to: "test.mp4".to_string(),
///     raw_log_message: line.to_string()
/// }));
/// ```
pub fn try_parse_output(mut s: &str) -> Option<FfmpegOutput> {
  let raw_log_message = s.to_string();

  s = s
    .strip_prefix("[info]")
    .unwrap_or(s)
    .trim()
    .strip_prefix("Output #")?;

  let index = s
    .split_whitespace()
    .next()
    .and_then(|s| s.split(',').next())
    .and_then(|s| s.parse::<u32>().ok())?;

  let to = s.split(" to '").nth(1)?.split('\'').next()?.to_string();

  Some(FfmpegOutput {
    index,
    to,
    raw_log_message,
  })
}

/// Parses a line that represents a stream.
///
/// ## Examples
///
/// ### Video
///
/// #### Input stream
///
/// ```rust
/// use async_ffmpeg_sidecar::log_parser::try_parse_stream;
/// let line = "[info]   Stream #0:0: Video: wrapped_avframe, rgb24, 320x240 [SAR 1:1 DAR 4:3], 25 fps, 25 tbr, 25 tbn\n";
/// let stream = try_parse_stream(line).unwrap();
/// assert_eq!(stream.format, "wrapped_avframe");;
/// assert_eq!(stream.language, "");
/// assert_eq!(stream.parent_index, 0);
/// assert_eq!(stream.stream_index, 0);
/// assert!(stream.is_video());
///
/// let video_data = stream.video_data().unwrap();
/// assert_eq!(video_data.pix_fmt, "rgb24");
/// assert_eq!(video_data.width, 320);
/// assert_eq!(video_data.height, 240);
/// assert_eq!(video_data.fps, 25.0);
/// ```
///
///  #### Output stream
///
/// ```rust
///  use async_ffmpeg_sidecar::log_parser::try_parse_stream;
///  let line = "[info]   Stream #1:5(eng): Video: h264 (avc1 / 0x31637661), yuv444p(tv, progressive), 320x240 [SAR 1:1 DAR 4:3], q=2-31, 25 fps, 12800 tbn\n";
///  let stream = try_parse_stream(line).unwrap();
///  assert_eq!(stream.format, "h264");
///  assert_eq!(stream.language, "eng");
///  assert_eq!(stream.parent_index, 1);
///  assert_eq!(stream.stream_index, 5);
///  assert!(stream.is_video());
///  let video_data = stream.video_data().unwrap();
///  assert_eq!(video_data.pix_fmt, "yuv444p");
///  assert_eq!(video_data.width, 320);
///  assert_eq!(video_data.height, 240);
///  assert_eq!(video_data.fps, 25.0);
///  ```
///
/// ### Audio
///
/// #### Input Stream
///
/// ```rust
/// use async_ffmpeg_sidecar::log_parser::try_parse_stream;
/// let line = "[info]   Stream #0:1(eng): Audio: opus, 48000 Hz, stereo, fltp (default)\n";
/// let stream = try_parse_stream(line).unwrap();
/// assert_eq!(stream.format, "opus");
/// assert_eq!(stream.language, "eng");
/// assert_eq!(stream.parent_index, 0);
/// assert_eq!(stream.stream_index, 1);
/// assert!(stream.is_audio());
/// let audio_data = stream.audio_data().unwrap();
/// assert_eq!(audio_data.sample_rate, 48000);
/// assert_eq!(audio_data.channels, "stereo");
/// ```
///
/// ```rust
/// use async_ffmpeg_sidecar::log_parser::try_parse_stream;
/// let line = "[info]   Stream #3:10(ger): Audio: dts (DTS-HD MA), 48000 Hz, 7.1, s32p (24 bit)\n";
/// let stream = try_parse_stream(line).unwrap();
/// assert_eq!(stream.format, "dts");
/// assert_eq!(stream.language, "ger");
/// assert_eq!(stream.parent_index, 3);
/// assert_eq!(stream.stream_index, 10);
/// assert!(stream.is_audio());
/// let audio_data = stream.audio_data().unwrap();
/// assert_eq!(audio_data.sample_rate, 48000);
/// assert_eq!(audio_data.channels, "7.1");
/// ```
///
/// ### Output stream
///
/// ```rust
/// use async_ffmpeg_sidecar::log_parser::try_parse_stream;
/// let line = "[info]   Stream #10:1: Audio: mp2, 44100 Hz, mono, s16, 384 kb/s\n";
/// let stream = try_parse_stream(line).unwrap();
/// assert_eq!(stream.format, "mp2");
/// assert_eq!(stream.language, "");
/// assert_eq!(stream.parent_index, 10);
/// assert_eq!(stream.stream_index, 1);
/// assert!(stream.is_audio());
/// let audio_data = stream.audio_data().unwrap();
/// assert_eq!(audio_data.sample_rate, 44100);
/// assert_eq!(audio_data.channels, "mono");
/// ```
///
/// ### Subtitle
///
/// #### Input Stream
///
/// ```rust
/// use async_ffmpeg_sidecar::log_parser::try_parse_stream;
/// let line = "[info]   Stream #0:4(eng): Subtitle: ass (default) (forced)\n";
/// let stream = try_parse_stream(line).unwrap();
/// assert_eq!(stream.format, "ass");
/// assert_eq!(stream.language, "eng");
/// assert_eq!(stream.parent_index, 0);
/// assert_eq!(stream.stream_index, 4);
/// assert!(stream.is_subtitle());
/// ```
///
/// ```rust
/// use async_ffmpeg_sidecar::log_parser::try_parse_stream;
/// let line = "[info]   Stream #0:13(dut): Subtitle: hdmv_pgs_subtitle, 1920x1080\n";
/// let stream = try_parse_stream(line).unwrap();
/// assert_eq!(stream.format, "hdmv_pgs_subtitle");
/// assert_eq!(stream.language, "dut");
/// assert_eq!(stream.parent_index, 0);
/// assert_eq!(stream.stream_index, 13);
/// assert!(stream.is_subtitle());
/// ```
/// ### Other
///
/// #### Input Stream
///
/// ```rust
/// use async_ffmpeg_sidecar::log_parser::try_parse_stream;
/// let line = "[info]   Stream #0:2(und): Data: none (rtp  / 0x20707472), 53 kb/s (default)\n";
/// let stream = try_parse_stream(line).unwrap();
/// assert_eq!(stream.format, "none");
/// assert_eq!(stream.language, "und");
/// assert_eq!(stream.parent_index, 0);
/// assert_eq!(stream.stream_index, 2);
/// assert!(stream.is_other());
/// ```
///
/// ```rust
/// use async_ffmpeg_sidecar::log_parser::try_parse_stream;
/// let line = "[info]   Stream #0:2[0x3](eng): Data: bin_data (text / 0x74786574)\n";
/// let stream = try_parse_stream(line).unwrap();
/// assert_eq!(stream.format, "bin_data");
/// assert_eq!(stream.language, "eng");
/// assert_eq!(stream.parent_index, 0);
/// assert_eq!(stream.stream_index, 2);
/// assert!(stream.is_other());
/// ```
pub fn try_parse_stream(s: &str) -> Option<FfmpegStream> {
  let raw_log_message = s.to_string();

  let s = s
    .strip_prefix("[info]")
    .unwrap_or(s)
    .trim()
    .strip_prefix("Stream #")?;
  let mut comma_iter = CommaIter::new(s);
  let mut colon_iter = comma_iter.next()?.split(':');

  let parent_index = colon_iter.next()?.parse::<u32>().ok()?;

  // Here handle the pattern such as `2[0x3](eng)`
  let indices_nad_maybe_language = colon_iter
    .next()?
    // Remove everything inside and including square brackets
    .split(['[', ']'])
    .step_by(2)
    .collect::<String>();
  let mut paranthesis_iter = indices_nad_maybe_language.split('(');
  let stream_index = paranthesis_iter.next()?.trim().parse::<u32>().ok()?;
  let language = paranthesis_iter.next().map_or("".to_string(), |lang| {
    lang.trim_end_matches(')').to_string()
  });

  // Here handle the pattern such as `Video: av1 (Main)`
  let stream_type = colon_iter.next()?.trim();
  let format = colon_iter
    .next()?
    .trim()
    .split(&[' ', '(']) // trim trailing junk like `(Main)`
    .next()?
    .to_string();

  // For audio and video handle remaining string in specialized functions.
  let type_specific_data: StreamTypeSpecificData = match stream_type {
    "Audio" => try_parse_audio_stream(comma_iter)?,
    "Subtitle" => StreamTypeSpecificData::Subtitle,
    "Video" => try_parse_video_stream(comma_iter)?,
    _ => StreamTypeSpecificData::Other,
  };

  Some(FfmpegStream {
    format,
    language,
    parent_index,
    stream_index,
    raw_log_message,
    type_specific_data,
  })
}

/// Parses the log output part that is specific to audio streams.
fn try_parse_audio_stream(mut comma_iter: CommaIter) -> Option<StreamTypeSpecificData> {
  let sample_rate = comma_iter
    .next()?
    .split_whitespace()
    .next()?
    .parse::<u32>()
    .ok()?;

  let channels = comma_iter.next()?.trim().to_string();

  Some(StreamTypeSpecificData::Audio(AudioStream {
    sample_rate,
    channels,
  }))
}

/// Parses the log output part that is specific to video streams.
fn try_parse_video_stream(mut comma_iter: CommaIter) -> Option<StreamTypeSpecificData> {
  let pix_fmt = comma_iter
    .next()?
    .trim()
    .split(&[' ', '(']) // trim trailing junk like "(tv, progressive)"
    .next()?
    .to_string();

  let dims = comma_iter.next()?.split_whitespace().next()?;
  let mut dims_iter = dims.split('x');
  let width = dims_iter.next()?.parse::<u32>().ok()?;
  let height = dims_iter.next()?.parse::<u32>().ok()?;

  // FPS does not have to be the next part, so we iterate until we find it. There is nothing else we
  // are interested in at this point, so its OK to skip anything in-between.
  let fps = comma_iter
    .find_map(|part| {
      if part.trim().ends_with("fps") {
        part.split_whitespace().next()
      } else {
        None
      }
    })
    .and_then(|fps_str| fps_str.parse::<f32>().ok())?;

  Some(StreamTypeSpecificData::Video(VideoStream {
    pix_fmt,
    width,
    height,
    fps,
  }))
}

/// Parse a progress update line from ffmpeg.
///
/// ## Example
/// ```rust
/// use async_ffmpeg_sidecar::log_parser::try_parse_progress;
/// let line = "[info] frame= 1996 fps=1984 q=-1.0 Lsize=     372kB time=00:01:19.72 bitrate=  38.2kbits/s speed=79.2x\n";
/// let progress = try_parse_progress(line).unwrap();
/// assert_eq!(progress.frame, 1996);
/// assert_eq!(progress.fps, 1984.0);
/// assert_eq!(progress.q, -1.0);
/// assert_eq!(progress.size_kb, 372);
/// assert_eq!(progress.time, "00:01:19.72");
/// assert_eq!(progress.bitrate_kbps, 38.2);
/// assert_eq!(progress.speed, 79.2);
/// ```
pub fn try_parse_progress(mut string: &str) -> Option<FfmpegProgress> {
  let raw_log_message = string.to_string();

  string = string.strip_prefix("[info]").unwrap_or(string).trim();

  let frame = string
    .split("frame=")
    .nth(1)?
    .split_whitespace()
    .next()?
    .parse::<u32>()
    .ok()?;
  let fps = string
    .split("fps=")
    .nth(1)?
    .split_whitespace()
    .next()?
    .parse::<f32>()
    .ok()?;
  let q = string
    .split("q=")
    .nth(1)?
    .split_whitespace()
    .next()?
    .parse::<f32>()
    .ok()?;
  let size_kb = string
    .split("size=") // captures "Lsize=" AND "size="
    .nth(1)?
    .split_whitespace()
    .next()
    .map(|s| s.trim())
    .and_then(|s| {
      s.strip_suffix("KiB") // FFmpeg v7.0 and later
        .or_else(|| s.strip_suffix("kB")) // FFmpeg v6.0 and prior
        .or_else(|| s.ends_with("N/A").then(|| "0")) // handles "N/A"
    })?
    .parse::<u32>()
    .ok()?;
  let time = string
    .split("time=")
    .nth(1)?
    .split_whitespace()
    .next()?
    .to_string();
  let bitrate_kbps = string
    .split("bitrate=")
    .nth(1)?
    .split_whitespace()
    .next()?
    .trim()
    .replace("kbits/s", "")
    .parse::<f32>()
    .unwrap_or(0.0); // handles "N/A"
  let speed = string
    .split("speed=")
    .nth(1)?
    .split_whitespace()
    .next()?
    .strip_suffix('x')
    .map(|s| s.parse::<f32>().unwrap_or(0.0))
    .unwrap_or(0.0);

  Some(FfmpegProgress {
    frame,
    fps,
    q,
    size_kb,
    time,
    bitrate_kbps,
    speed,
    raw_log_message,
  })
}

/// Parse a time string in the format `HOURS:MM:SS.MILLISECONDS` into a number of seconds.
///
/// <https://trac.ffmpeg.org/wiki/Seeking#Time-unit>
///
/// ## Examples
///
/// ```rust
/// use async_ffmpeg_sidecar::log_parser::parse_time_str;
/// assert_eq!(parse_time_str("00:00:00.00"), Some(0.0));
/// assert_eq!(parse_time_str("5"), Some(5.0));
/// assert_eq!(parse_time_str("0.123"), Some(0.123));
/// assert_eq!(parse_time_str("1:00.0"), Some(60.0));
/// assert_eq!(parse_time_str("1:01.0"), Some(61.0));
/// assert_eq!(parse_time_str("1:01:01.123"), Some(3661.123));
/// assert_eq!(parse_time_str("N/A"), None);
/// ```
pub fn parse_time_str(s: &str) -> Option<f64> {
  let mut seconds = 0.0;

  let mut smh = s.split(':').rev();
  if let Some(sec) = smh.next() {
    seconds += sec.parse::<f64>().ok()?;
  }

  if let Some(min) = smh.next() {
    seconds += min.parse::<f64>().ok()? * 60.0;
  }

  if let Some(hrs) = smh.next() {
    seconds += hrs.parse::<f64>().ok()? * 60.0 * 60.0;
  }

  Some(seconds)
}

#[cfg(test)]
mod tests {
  use super::*;
  // use crate::{command::BackgroundCommand, paths::ffmpeg_path};
  use crate::command::BackgroundCommand;
  use crate::paths::ffmpeg_path;
  use std::io::{Cursor, Seek, SeekFrom, Write};
  use std::process::Stdio;
  use tokio::process::Command;

  #[tokio::test]
  async fn test_parse_version() {
    let cmd = Command::new(ffmpeg_path())
      .create_no_window()
      .arg("-version")
      .stdout(Stdio::piped())
      // ⚠ notice that ffmpeg emits on stdout when `-version` or `-help` is passed!
      .spawn()
      .unwrap();

    let stdout = cmd.stdout.unwrap();
    let reader = BufReader::new(stdout);
    let mut parser = FfmpegLogParser::new(reader);
    while let Ok(event) = parser.parse_next_event().await {
      if let FfmpegEvent::ParsedVersion(_) = event {
        return;
      }
    }
    panic!() // should have found a version
  }

  #[tokio::test]
  async fn test_parse_configuration() {
    let cmd = Command::new(ffmpeg_path())
      .create_no_window()
      .arg("-version")
      .stdout(Stdio::piped())
      // ⚠ notice that ffmpeg emits on stdout when `-version` or `-help` is passed!
      .spawn()
      .unwrap();

    let stdout = cmd.stdout.unwrap();
    let reader = BufReader::new(stdout);
    let mut parser = FfmpegLogParser::new(reader);
    while let Ok(event) = parser.parse_next_event().await {
      if let FfmpegEvent::ParsedConfiguration(_) = event {
        return;
      }
    }
    panic!() // should have found a configuration
  }

  /// Test case from https://github.com/nathanbabcock/ffmpeg-sidecar/issues/2#issue-1606661255
  #[tokio::test]
  async fn test_macos_line_endings() {
    let stdout_str = "[info] ffmpeg version N-109875-geabc304d12-tessus  https://evermeet.cx/ffmpeg/  Copyright (c) 2000-2023 the FFmpeg developers\n[info]   built with Apple clang version 11.0.0 (clang-1100.0.33.17)\n[info]   configuration: --cc=/usr/bin/clang --prefix=/opt/ffmpeg --extra-version=tessus --enable-avisynth --enable-fontconfig --enable-gpl --enable-libaom --enable-libass --enable-libbluray --enable-libdav1d --enable-libfreetype --enable-libgsm --enable-libmodplug --enable-libmp3lame --enable-libmysofa --enable-libopencore-amrnb --enable-libopencore-amrwb --enable-libopenh264 --enable-libopenjpeg --enable-libopus --enable-librubberband --enable-libshine --enable-libsnappy --enable-libsoxr --enable-libspeex --enable-libtheora --enable-libtwolame --enable-libvidstab --enable-libvmaf --enable-libvo-amrwbenc --enable-libvorbis --enable-libvpx --enable-libwebp --enable-libx264 --enable-libx265 --enable-libxavs --enable-libxvid --enable-libzimg --enable-libzmq --enable-libzvbi --enable-version3 --pkg-config-flags=--static --disable-ffplay\n[info]   libavutil      58.  1.100 / 58.  1.100\n[info]   libavcodec     60.  2.100 / 60.  2.100\n[info]   libavformat    60.  2.100 / 60.  2.100\n[info]   libavdevice    60.  0.100 / 60.  0.100\n[info]   libavfilter     9.  2.100 /  9.  2.100\n[info]   libswscale      7.  0.100 /  7.  0.100\n[info]   libswresample   4.  9.100 /  4.  9.100\n[info]   libpostproc    57.  0.100 / 57.  0.100\n[info] Input #0, lavfi, from 'testsrc=duration=10':\n[info]   Duration: N/A, start: 0.000000, bitrate: N/A\n[info]   Stream #0:0: Video: wrapped_avframe, rgb24, 320x240 [SAR 1:1 DAR 4:3], 25 fps, 25 tbr, 25 tbn\n[info] Stream mapping:\n[info]   Stream #0:0 -> #0:0 (wrapped_avframe (native) -> rawvideo (native))\n[info] Press [q] to stop, [?] for help\n[info] Output #0, rawvideo, to 'pipe:':\n[info]   Metadata:\n[info]     encoder         : Lavf60.2.100\n[info]   Stream #0:0: Video: rawvideo (RGB[24] / 0x18424752), rgb24(progressive), 320x240 [SAR 1:1 DAR 4:3], q=2-31, 46080 kb/s, 25 fps, 25 tbn\n[info]     Metadata:\n[info]       encoder         : Lavc60.2.100 rawvideo\n[info] frame=    0 fps=0.0 q=0.0 size=       0kB time=-577014:32:22.77 bitrate=  -0.0kbits/s speed=N/A";

    // Emulate a stderr channel
    let mut cursor = Cursor::new(Vec::new());
    cursor.write_all(stdout_str.as_bytes()).unwrap();
    cursor.seek(SeekFrom::Start(0)).unwrap();

    let reader = BufReader::new(cursor);
    let mut parser = FfmpegLogParser::new(reader);
    let mut num_events = 0;
    while let Ok(event) = parser.parse_next_event().await {
      match event {
        FfmpegEvent::LogEOF => break,
        _ => num_events += 1,
      }
    }
    assert!(num_events > 1);
  }

  /// Test case for https://github.com/nathanbabcock/ffmpeg-sidecar/issues/31
  /// Covers regression in progress parsing introduced in FFmpeg 7.0
  /// The string format for `Lsize` units went from `kB` to `KiB`
  #[test]
  fn test_parse_progress_v7() {
    let line = "[info] frame=    5 fps=0.0 q=-1.0 Lsize=      10KiB time=00:00:03.00 bitrate=  27.2kbits/s speed= 283x\n";
    let progress = try_parse_progress(line).unwrap();
    assert_eq!(progress.frame, 5);
    assert_eq!(progress.fps, 0.0);
    assert_eq!(progress.q, -1.0);
    assert_eq!(progress.size_kb, 10);
    assert_eq!(progress.time, "00:00:03.00");
    assert_eq!(progress.bitrate_kbps, 27.2);
    assert_eq!(progress.speed, 283.0);
  }

  #[test]
  fn test_parse_progress_no_size() {
    let line = "[info] frame=  163 fps= 13 q=4.4 size=N/A time=00:13:35.00 bitrate=N/A speed=64.7x";
    let progress = try_parse_progress(line).unwrap();
    assert!(progress.frame == 163);
    assert!(progress.fps == 13.0);
    assert!(progress.q == 4.4);
    assert!(progress.size_kb == 0);
    assert!(progress.time == "00:13:35.00");
    assert!(progress.bitrate_kbps == 0.0);
    assert!(progress.speed == 64.7);
  }

  /// Check for handling first progress message w/ bitrate=N/A and speed=N/A
  /// These never appeared on Windows but showed up on Ubuntu and MacOS
  #[test]
  fn test_parse_progress_empty() {
    let line =
      "[info] frame=    0 fps=0.0 q=-0.0 size=       0kB time=00:00:00.00 bitrate=N/A speed=N/A\n";
    let progress = try_parse_progress(line).unwrap();
    assert_eq!(progress.frame, 0);
    assert_eq!(progress.fps, 0.0);
    assert_eq!(progress.q, -0.0);
    assert_eq!(progress.size_kb, 0);
    assert_eq!(progress.time, "00:00:00.00");
    assert_eq!(progress.bitrate_kbps, 0.0);
    assert_eq!(progress.speed, 0.0);
  }
}
