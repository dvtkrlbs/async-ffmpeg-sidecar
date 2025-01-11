use async_ffmpeg_sidecar::ffprobe::ffprobe_version;

#[cfg(feature = "download_ffmpeg")]
#[tokio::main]
async fn main() {
  use async_ffmpeg_sidecar::download::auto_download;

  println!("Downloading ffprobe");
  // Download ffprobe from a configured source.
  // Note that not all distributions include ffprobe in their bundle.
  auto_download().await.unwrap();

  println!("Downloaded ffprobe");

  // Try running the executable and printing the version number.
  let version = ffprobe_version().await.unwrap();
  println!("ffprobe version: {}", version);
}

#[cfg(not(feature = "download_ffmpeg"))]
fn main() {
  eprintln!(r#"This example requires the "download_ffmpeg" feature to be enabled."#);
  println!("The feature is included by default unless manually disabled.");
  println!("Please run `cargo run --example download_ffmpeg`.");
}
