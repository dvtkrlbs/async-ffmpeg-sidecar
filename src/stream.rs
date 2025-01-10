//! A stream of events from an Ffmpeg process.

use crate::event::{FfmpegProgress, LogLevel};
use crate::{
  child::FfmpegChild, event::FfmpegEvent, log_parser::FfmpegLogParser, metadata::FfmpegMetadata,
};
use anyhow::Context;
use futures_util::{Stream, StreamExt};
use std::future::Future;
use std::pin::Pin;
use std::task::Poll;
use tokio::{io::BufReader, pin, process::ChildStderr};

pub struct FfmpegEventStream {
  metadata: FfmpegMetadata,
  // stderr: ChildStderr,
  log_parser: FfmpegLogParser<BufReader<ChildStderr>>,
  // stdout: Option<ChildStdout>,
  // err: bool,
}

impl FfmpegEventStream {
  pub fn new(child: &mut FfmpegChild) -> anyhow::Result<Self> {
    let stderr = child.take_stderr().context("no stderr channel")?;
    let reader = BufReader::new(stderr);
    let parser = FfmpegLogParser::new(reader);
    // let stdout = child.take_stdout();

    Ok(Self {
      metadata: FfmpegMetadata::new(),
      log_parser: parser,
      // stdout,
      // err: false,
    })
  }

  pub async fn collect_metadata(&mut self) -> anyhow::Result<FfmpegMetadata> {
    let mut event_queue: Vec<FfmpegEvent> = Vec::new();

    while !self.metadata.is_completed() {
      let event = self.next().await;
      match event {
        Some(e) => event_queue.push(e),
        None => {
          let errors = event_queue
            .iter()
            .filter_map(|e| match e {
              FfmpegEvent::Error(e) | FfmpegEvent::Log(LogLevel::Error, e) => Some(e.to_string()),
              _ => None,
            })
            .collect::<Vec<String>>()
            .join("");

          anyhow::bail!(
            "Stream ran out before metadata was gathered. The following errors occurred: {errors}"
          )
        }
      }
    }

    Ok(self.metadata.clone())
  }

  //// Stream filters

  /// Returns a stream over error messages (`FfmpegEvent::Error` and `FfmpegEvent::LogError`).
  pub fn filter_errors(self) -> impl Stream<Item = String> {
    self.filter_map(|event| {
      futures::future::ready(match event {
        FfmpegEvent::Error(e) | FfmpegEvent::Log(LogLevel::Error, e) => Some(e),
        _ => None,
      })
    })
  }

  /// Filter out all events except for progress (`FfmpegEvent::Progress`).
  pub fn filter_progress(self) -> impl Stream<Item = FfmpegProgress> {
    self.filter_map(|event| {
      futures::future::ready(match event {
        FfmpegEvent::Progress(p) => Some(p),
        _ => None,
      })
    })
  }
}

impl Stream for FfmpegEventStream {
  type Item = FfmpegEvent;

  fn poll_next(
    mut self: Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
  ) -> Poll<Option<FfmpegEvent>> {
    let fut = self.log_parser.parse_next_event();
    let item = {
      pin!(fut);

      match fut.poll(cx) {
        Poll::Ready(Ok(event)) => {
          if event == FfmpegEvent::LogEOF {
            return Poll::Ready(None);
          }

          event
        }
        Poll::Ready(Err(e)) => return Poll::Ready(Some(FfmpegEvent::Error(e.to_string()))),
        Poll::Pending => return Poll::Pending,
      }
    };

    if !self.metadata.is_completed() {
      if let Err(e) = self.metadata.handle_event(&item) {
        return Poll::Ready(Some(FfmpegEvent::Error(e.to_string())));
      }
    }

    Poll::Ready(Some(item))
  }
}
