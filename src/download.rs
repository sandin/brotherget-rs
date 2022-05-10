use std::io::SeekFrom;
use indicatif::{MultiProgress, ProgressBar};
use futures::StreamExt;
use tokio::fs::{remove_file, File};
use tokio::io::{AsyncSeekExt, AsyncWriteExt, AsyncReadExt};
use reqwest::header::{ACCEPT_RANGES, CONTENT_LENGTH, RANGE};
use reqwest::Response;
use reqwest::StatusCode;
use crate::error::BError;

#[derive(Debug)]
pub struct DownloadRequest {
  pub url: String,
  pub range: (u64, u64),
  pub size: u64,
  pub filename: String,
  pub proxy_url: Option<String>,
  pub proxy_name: String,
}

pub async fn download(request: DownloadRequest, _: MultiProgress, progress_bar: ProgressBar) -> Result<DownloadRequest, BError> {
  let mut output_file = File::create(&request.filename).await?;

  let mut builder = reqwest::Client::builder();
  let agent_name; 
  if let Some(ref proxy_url) = request.proxy_url {
    let proxy = reqwest::Proxy::all(proxy_url)?;
    builder = builder.proxy(proxy);
    agent_name = format!("{}", &request.proxy_name);
  } else {
    agent_name = "localhost".to_string();
  }
  progress_bar.set_message(format!("{} {}-{}", agent_name, request.range.0, request.range.1));

  let client = builder.build()?; 
  let range = format!("bytes={}-{}", request.range.0, request.range.1);
  let response: Response = client.get(&request.url).header(RANGE, range).send().await?;
  let status = response.status();
  if !(status == StatusCode::OK || status == StatusCode::PARTIAL_CONTENT) {
    return Err(BError::Download(format!("Unexpected server response: {}", status)));
  }
  //println!("response headers={:#?}", response.headers());

  let mut stream = response.bytes_stream();
  while let Some(item) = stream.next().await {
    let mut chunk = item?;
    progress_bar.inc(chunk.len() as u64);
    output_file.write_all_buf(&mut chunk).await?;
  }
  //println!("downloaded url={}, filename={}", &request.url, &request.filename);
  progress_bar.finish_with_message("done");

  Ok(request)
}

pub async fn head(url: &str) -> Result<(bool, u64), BError> {
  let client = reqwest::Client::builder().build()?;
  let resp = client.head(url).send().await?;
  let headers = resp.headers();
  //println!("url: {}, headers: {:#?}", url, headers);

  let support_range: bool = headers.contains_key(ACCEPT_RANGES);
  let content_length: u64 = headers.get(CONTENT_LENGTH).unwrap().to_str()?.parse().unwrap(); // TODO: match

  Ok((support_range, content_length))
}

pub async fn merge_files(requests: Vec<DownloadRequest>, output_filename: &str) -> Result<String, BError> {
  let mut file = File::create(output_filename).await?;

  let mut requests = requests;
  requests.sort_by(|a, b| a.filename.cmp(&b.filename));
  for request in requests {
    let mut part_file = File::open(&request.filename).await?;
    let mut buffer = Vec::new();
    let n: usize = part_file.read_to_end(&mut buffer).await?;  // TODO: stream read, buffer write
    file.seek(SeekFrom::Start(request.range.0)).await?; 
    //println!("copy file {} -> {} at {}, bytes {}", &request.filename, &output_filename, request.range.0, n);
    file.write_all(&buffer[..n]).await?;

    remove_file(&request.filename).await?;
  }

  Ok(output_filename.to_string())
}
