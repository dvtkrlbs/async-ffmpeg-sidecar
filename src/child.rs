//! Wrapper around `tokio::process` containing a spawned Ffmpeg command.

use std::process::ExitStatus;
use tokio::{
    io::{self},
    process::{Child, ChildStderr, ChildStdin, ChildStdout},
};

use crate::stream::FfmpegEventStream;
use anyhow::Context;
use tokio::io::AsyncWriteExt;

/// A wrapper around [`tokio::process::Child`] containing a spawned Ffmpeg command.
/// Provides interfaces for reading parsed metadata, progress updates, warnings and errors and
/// piped output frames if applicable.
pub struct FfmpegChild {
    inner: Child,
}

impl FfmpegChild {
    /// Crates a stream over events emitted by Ffmpeg. Functions similarly to
    /// `Lines` from [`tokio::io::BufReader`], but providing a variety of parsed events:
    /// - Log messages
    /// - Parsed metadata
    /// - Progress updates
    /// - Errors and warnings
    /// - Raw output frames
    pub fn stream(&mut self) -> anyhow::Result<FfmpegEventStream> {
        FfmpegEventStream::new(self)
    }

    /// Escape hatch to manually control the process' stdout channel.
    /// Calling this method takes ownership of the stdout channel, so
    /// the iterator will no longer include output frames in the stream of events.
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.inner.stdout.take()
    }

    /// Escape hatch to manually control the process' stderr channel.
    /// This method is mutually exclusive with `events_iter`, which relies on
    /// the stderr channel to parse events.
    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.inner.stderr.take()
    }

    /// Escape hatch to manually control the process' stdin channel.
    /// This method is mutually exclusive with `send_stdin_command` and `quit`,
    /// which use the stdin channel to send commands to ffmpeg.
    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.inner.stdin.take()
    }

    /// Send a command to ffmpeg over stdin, used during interactive mode.
    ///
    /// This method does not validate that the command is expected or handled
    /// correctly by ffmpeg. The returned `io::Result` indicates only whether the
    /// command was successfully sent or not.
    ///
    /// In a typical ffmpeg build, these are the supported commands:
    ///
    /// ```txt
    /// ?      show this help
    /// +      increase verbosity
    /// -      decrease verbosity
    /// c      Send command to first matching filter supporting it
    /// C      Send/Queue command to all matching filters
    /// D      cycle through available debug modes
    /// h      dump packets/hex press to cycle through the 3 states
    /// q      quit
    /// s      Show QP histogram
    /// ```
    pub async fn send_stdin_command(&mut self, command: &[u8]) -> anyhow::Result<()> {
        let mut stdin = self.inner.stdin.take().context("Missing child stdin")?;
        stdin.write_all(command).await?;
        self.inner.stdin.replace(stdin);
        Ok(())
    }

    /// Send a `q` command to ffmpeg over stdin,
    /// requesting a graceful shutdown as soon as possible.
    ///
    /// This method returns after the command has been sent; the actual shut down
    /// may take a few more frames as ffmpeg flushes its buffers and writes the
    /// trailer, if applicable.
    pub async fn quit(&mut self) -> anyhow::Result<()> {
        self.send_stdin_command(b"q").await
    }

    /// Forcibly terminate the inner child process.
    ///
    /// Alternatively, you may choose to gracefully stop the child process by
    /// sending a command over stdin, using the `quit` method.
    ///
    /// Identical to `kill` in [`std::process::Child`].
    pub async fn kill(&mut self) -> io::Result<()> {
        self.inner.kill().await
    }

    /// Waits for the inner child process to finish execution.
    ///
    /// Identical to `wait` in [`std::process::Child`].
    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        self.inner.wait().await
    }

    /// Wrap a [`std::process::Child`] in a `FfmpegChild`. Should typically only
    /// be called by `FfmpegCommand::spawn`.
    ///
    /// ## Panics
    ///
    /// Panics if any of the child process's stdio channels were not piped.
    /// This could be because ffmpeg was spawned with `-nostdin`, or if the
    /// `Child` instance was not configured with `stdin(Stdio::piped())`.
    pub(crate) fn from_inner(inner: Child) -> Self {
        assert!(inner.stdin.is_some(), "stdin was not piped");
        assert!(inner.stdout.is_some(), "stdout was not piped");
        assert!(inner.stderr.is_some(), "stderr was not piped");
        Self { inner }
    }

    /// Escape hatch to access the inner `Child`.
    pub fn as_inner(&mut self) -> &Child {
        &self.inner
    }

    /// Escape hatch to mutably access the inner `Child`.
    pub fn as_inner_mut(&mut self) -> &mut Child {
        &mut self.inner
    }
}
