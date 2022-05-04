use error_chain::error_chain;
use futures::future::join_all;
use futures::StreamExt;
use reqwest::header::{HeaderValue, ACCEPT_RANGES, CONTENT_LENGTH, RANGE};
use reqwest::Response;
use reqwest::StatusCode;
use std::collections::HashMap;
use tokio::fs::{remove_file, File};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use indicatif::{ProgressBar, MultiProgress};

error_chain! {
  foreign_links {
    Io(std::io::Error);
    Reqwest(reqwest::Error);
    Header(reqwest::header::ToStrError);
  }
  errors {
    MyError(t: String) {
      description("MyError")
      display("MyError: '{}'", t)
    }
  }
}

struct DownloadRequest {
  url: String,
  range: (u64, u64),
  size: u64,
  filename: String,
}

async fn download(request: DownloadRequest) -> Result<()> {
  println!("download url={}, bytes={}-{}, size={}, filename={}", &request.url, request.range.0, request.range.1, request.size, &request.filename);
  let progress_bar = ProgressBar::new(request.size);

  let mut output_file = File::create(&request.filename).await?;

  let client = reqwest::Client::builder().build()?;
  let range = format!("bytes={}-{}", request.range.0, request.range.1);
  let response: Response = client.get(&request.url).header(RANGE, range).send().await?;
  let status = response.status();
  if !(status == StatusCode::OK || status == StatusCode::PARTIAL_CONTENT) {
    error_chain::bail!("Unexpected server response: {}", status)
  }

  let mut stream = response.bytes_stream();
  let mut downloaded: u64 = 0;
  while let Some(item) = stream.next().await {
    let mut chunk = item?;
    //downloaded = downloaded + (chunk.len() as u64);
    //progress_bar.set_position(downloaded);
    progress_bar.inc(chunk.len() as u64);
    output_file.write_all_buf(&mut chunk).await?;
  }
  println!("download url={}, filename={}", &request.url, &request.filename);
  progress_bar.finish();

  Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
  let url = "https://releases.ubuntu.com/22.04/ubuntu-22.04-desktop-amd64.iso";

  let proxy_count = 5;

  let client = reqwest::Client::builder().build()?;
  let resp = client.head(url).send().await?;
  let headers = resp.headers();

  let chunk_count: u32 = proxy_count;
  if headers.contains_key(ACCEPT_RANGES) && headers.contains_key(CONTENT_LENGTH) {
    let content_length: u64 = headers
      .get(CONTENT_LENGTH)
      .unwrap()
      .to_str()?
      .parse()
      .unwrap();
    let chunk_size: u64 = (content_length as f64 / proxy_count as f64).ceil() as u64;
    let last_chunk_size: u64 = content_length % chunk_size;
    let total_content_length = (chunk_size * (chunk_count - 1) as u64) + last_chunk_size;
    assert!(total_content_length == content_length);
    println!(
      "content_length: {}, chunk_count: {}, chunk_length: {}, last_chunk_length: {}",
      content_length, chunk_count, chunk_size, last_chunk_size
    );

    let multiProgress = MultiProgress::new();

    println!("starting download...");
    let mut handles = vec![];
    let mut offset = 0;
    for i in 0..chunk_count {
      let size = if i != chunk_count - 1 { chunk_size } else { last_chunk_size };
      let end = offset + size;
      let req = DownloadRequest {
        url: url.to_string(),
        range: (offset, end),
        size: size,
        filename: format!("output_filename.part{}", i)
      };

      handles.push(tokio::spawn(download(req)));
      offset = end + 1;
    }
    join_all(handles).await;
  }

  println!("{:#?}", headers);
  Ok(())
}
