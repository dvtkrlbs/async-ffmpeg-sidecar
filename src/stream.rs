//! A stream of events from an Ffmpeg process.

use crate::{
    child::FfmpegChild, event::FfmpegEvent, log_parser::FfmpegLogParser, metadata::FfmpegMetadata,
};
use anyhow::Context;
use futures_util::Stream;
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
