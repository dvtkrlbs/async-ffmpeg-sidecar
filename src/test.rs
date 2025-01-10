use crate::command::{ffmpeg_is_installed, FfmpegCommand};
use crate::event::FfmpegEvent;
use crate::version::ffmpeg_version;
use futures_util::stream::StreamExt;

fn approx_eq(a: f32, b: f32, error: f32) -> bool {
  (a - b).abs() < error
}

/// Returns `err` if the timeout task finishes before the FFmpeg

#[tokio::test]
async fn test_installed() {
  assert!(ffmpeg_is_installed().await);
}

#[tokio::test]
async fn test_version() {
  assert!(ffmpeg_version().await.is_ok())
}

#[tokio::test]
async fn test_progress() {
  let mut progress_events = 0;
  FfmpegCommand::new()
    .args("-f lavfi -i testsrc=duration=5:rate=1 -y output/test.mp4".split(' '))
    .spawn()
    .unwrap()
    .stream()
    .unwrap()
    .filter_progress()
    .for_each(|_| {
      progress_events += 1;
      futures::future::ready(())
    })
    .await;
  assert!(progress_events > 0);
}

#[tokio::test]
async fn test_error() {
  let errors = FfmpegCommand::new()
    // output format and pix_fmt are deliberately missing, cannot be inferred
    .args("-f lavfi -i testsrc=duration=1:rate=1 -".split(' '))
    .spawn()
    .unwrap()
    .stream()
    .unwrap()
    .filter_errors()
    .count()
    .await;

  assert!(errors > 0);
}

#[tokio::test]
async fn test_duration() {
  // Prepare the input file.
  // TODO construct this in-memory instead of writing to disk.
  FfmpegCommand::new()
    .args("-f lavfi -i testsrc=duration=5:rate=1 -y output/test_duration.mp4".split(' '))
    .spawn()
    .unwrap()
    .stream()
    .unwrap()
    .count()
    .await;

  let mut duration_received = false;

  FfmpegCommand::new()
    .input("output/test_duration.mp4")
    .format("mpegts")
    .pipe_stdout()
    .spawn()
    .unwrap()
    .stream()
    .unwrap()
    .for_each(|e| {
      futures::future::ready({
        if let FfmpegEvent::ParsedDuration(duration) = e {
          match duration_received {
            false => {
              assert_eq!(duration.duration, 5.0);
              duration_received = true
            }
            true => panic!("Received multiple duration events."),
          }
        }
      })
    })
    .await;

  assert!(duration_received);
}

#[tokio::test]
async fn test_metadata_duration() {
  // Prepare input file
  FfmpegCommand::new()
    .args("-f lavfi -i testsrc=duration=5:rate=1 -y output/test_metadata_duration.mp4".split(' '))
    .spawn()
    .unwrap()
    .stream()
    .unwrap()
    .count()
    .await;

  let mut child = FfmpegCommand::new()
    .input("output/test_metadata_duration.mp4")
    .format("mpegts")
    .pipe_stdout()
    .spawn()
    .unwrap();

  let metadata = child.stream().unwrap().collect_metadata().await.unwrap();
  child.kill().await.unwrap();

  assert_eq!(metadata.duration(), Some(5.0))
}

#[tokio::test]
async fn tset_kill_before_stream() {
  let mut child = FfmpegCommand::new().testsrc().rawvideo().spawn().unwrap();
  child.kill().await.unwrap();

  let vec = child.stream().unwrap().collect::<Vec<FfmpegEvent>>().await;

  assert_eq!(vec.len(), 0);
}

#[tokio::test]
async fn test_kill_after_stream() {
  let mut child = FfmpegCommand::new().testsrc().rawvideo().spawn().unwrap();
  let mut stream = child.stream().unwrap();
  assert!(stream.next().await.is_some());
  child.kill().await.unwrap();
  child.as_inner_mut().wait().await.unwrap();

  let count = stream
    .filter(|e| futures::future::ready(matches!(e, FfmpegEvent::Progress(_))))
    .count()
    .await;

  assert!(count <= 1);
}

#[tokio::test]
async fn test_quit() {
  let mut child = FfmpegCommand::new().testsrc().rawvideo().spawn().unwrap();
  child.quit().await.unwrap();
  let count = child.stream().unwrap().filter_progress().count().await;

  assert!(count <= 1);
}

// #[tokio::test]
// async fn test_overwrite_fallback() -> anyhow::Result<()> {
//   let output_path = "output/test_overwrite_fallback.jpg";
//   let timeout_ms = 1000;
//
//   // let write_file_with_timeout
//
//   todo!();
//   Ok(())
// }

#[tokio::test]
async fn test_overwrite() -> anyhow::Result<()> {
  let output_path = "output/test_overwrite.jpg";
  let write_file = || async {
    FfmpegCommand::new()
      .overwrite()
      .testsrc()
      .frames(1)
      .output(output_path)
      .spawn()?
      .wait()
      .await
  };

  write_file().await?;
  let time1 = tokio::fs::metadata(output_path).await?.modified()?;

  write_file().await?;
  let time2 = tokio::fs::metadata(output_path).await?.modified()?;

  assert_ne!(time1, time2);

  Ok(())
}

#[tokio::test]
async fn test_no_overwrite() -> anyhow::Result<()> {
  let output_path = "output/test_no_overwrite.jpg";

  let write_file = || async {
    FfmpegCommand::new()
      .no_overwrite()
      .testsrc()
      .frames(1)
      .output(output_path)
      .spawn()?
      .wait()
      .await
  };

  write_file().await?;
  let time1 = std::fs::metadata(output_path)?.modified()?;

  write_file().await?;
  let time2 = std::fs::metadata(output_path)?.modified()?;

  assert_eq!(time1, time2);

  Ok(())
}
