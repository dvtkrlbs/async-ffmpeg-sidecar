use async_ffmpeg_sidecar::command::FfmpegCommand;
use futures_util::stream::StreamExt;

#[tokio::main]
async fn main() {
  let mut child = FfmpegCommand::new()
    .arg("-report")
    .testsrc()
    .rawvideo()
    .print_command()
    .spawn()
    .unwrap();
  let count = child.stream().unwrap().filter_progress().count().await;

  assert!(count <= 1);
}
