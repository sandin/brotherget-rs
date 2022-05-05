use std::collections::HashMap;
use std::io::SeekFrom;
use std::time::{Duration, Instant};
use std::path::{Path, PathBuf};
use std::process;
use error_chain::error_chain;
use futures::future::join_all;
use futures::StreamExt;
use reqwest::header::{ACCEPT_RANGES, CONTENT_LENGTH, RANGE};
use reqwest::Response;
use reqwest::StatusCode;
use tokio::fs::{remove_file, File};
use tokio::io::{self, BufReader, AsyncSeekExt, AsyncWriteExt, AsyncReadExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle, HumanBytes};
use clap::{Arg, App};
use serde::Deserialize; 
use serde::Serialize;

error_chain! {
  foreign_links {
    Io(std::io::Error);
    Reqwest(reqwest::Error);
    Header(reqwest::header::ToStrError);
    JoinError(tokio::task::JoinError);
  }
  errors {
    MyError(t: String) {
      description("MyError")
      display("MyError: '{}'", t)
    }
  }
}

#[derive(Debug)]
struct DownloadRequest {
  url: String,
  range: (u64, u64),
  size: u64,
  filename: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct Server {
  server: String,
  server_port: i32,
  password: String,
  method: String,
  local_address: String,
  local_port: i32,
}

#[derive(Serialize, Deserialize, Debug)]
struct Config {
  servers: Vec<Server>
}

async fn download(request: DownloadRequest, multi_progress: MultiProgress, progress_bar: ProgressBar) -> Result<DownloadRequest> {
  progress_bar.set_message(format!("range={}-{}", request.range.0, request.range.1));

  let mut output_file = File::create(&request.filename).await?;

  let client = reqwest::Client::builder().build()?; // TODO: use proxy
  let range = format!("bytes={}-{}", request.range.0, request.range.1);
  let response: Response = client.get(&request.url).header(RANGE, range).send().await?;
  let status = response.status();
  if !(status == StatusCode::OK || status == StatusCode::PARTIAL_CONTENT) {
    error_chain::bail!("Unexpected server response: {}", status)
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

async fn head(url: &str) -> Result<(bool, u64)> {
  let client = reqwest::Client::builder().build()?;
  let resp = client.head(url).send().await?;
  let headers = resp.headers();
  //println!("url: {}, headers: {:#?}", url, headers);

  let support_range: bool = headers.contains_key(ACCEPT_RANGES);
  let content_length: u64 = headers.get(CONTENT_LENGTH).unwrap().to_str()?.parse().unwrap(); // TODO: match

  Ok((support_range, content_length))
}

async fn merge_files(requests: Vec<DownloadRequest>, output_filename: &str) -> Result<String> {
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

#[tokio::main]
async fn main() -> Result<()> {
  let mut app  = App::new("bget")
    .version(env!("CARGO_PKG_VERSION"))
    .arg(
      Arg::with_name("config")
          .long("config")
          .takes_value(false)
          .help("config json file"),
    )
    .arg(
      Arg::with_name("url").required(true).help("target url")
    );
  let mut matches = app.get_matches();
  let url = matches.value_of("url").unwrap();
  let config_filename = matches.value_of("config").unwrap_or("config.json");
  if !Path::new(config_filename).to_path_buf().exists() {
    eprintln!("Error: config file is missing.");
    process::exit(-1);
  }
  let mut config_file = File::open(config_filename).await?;
  let mut buffer = Vec::new();
  let n: usize = config_file.read_to_end(&mut buffer).await?;
  let config: Config = match serde_json::from_slice(&buffer[0..n]) {
    Ok(r) => r,
    Err(e) => {
      eprintln!("Error: can not parse config file {}.", config_filename);
      process::exit(-1);
    }
  };

  //let url = "https://releases.ubuntu.com/22.04/ubuntu-22.04-desktop-amd64.iso";
  //let url = "https://dl.google.com/go/go1.18.1.windows-amd64.msi"; // 132MB SHA256 Checksum: 9a3e9636eb5f9af4b3b29b116b7d28b8d6092c690ee4b88cfb9613b8851d4a66
  //let url = "https://golang.google.cn/dl/go1.18.1.src.tar.gz"; // 22MB SHA256 Checksum: efd43e0f1402e083b73a03d444b7b6576bb4c539ac46208b63a916b69aca4088
  let filename = match url.rfind("/") {
    Some(i) => &url[(i+1)..], // url = url.substring(url.lastIndexOf("/"))
    None => url
  };

  let chunk_count: u32 = config.servers.len() as u32;
  let (support_range, content_length) = head(url).await?;
  if support_range {
    let chunk_size: u64 = (content_length as f64 / chunk_count as f64).ceil() as u64;
    let last_chunk_size: u64 = content_length % chunk_size;
    let total_content_length = (chunk_size * (chunk_count - 1) as u64) + last_chunk_size;
    assert!(total_content_length == content_length);
    println!(
      "url: {}, content_length: {}, chunk_count: {}, chunk_length: {}, last_chunk_length: {}",
      url, content_length, chunk_count, chunk_size, last_chunk_size
    );

    let mut handles = vec![];

    let multi_progress = MultiProgress::new();
    let progress_style = ProgressStyle::with_template("[eta {eta_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {percent:3}% {binary_bytes_per_sec} {msg}")
      .unwrap()
      .progress_chars("##-");


    multi_progress.println("start downloading...");
    let start = Instant::now();
    let mut offset = 0;
    for i in 0..chunk_count {
      let end = if i != chunk_count - 1 { offset + chunk_size - 1 } else { offset + last_chunk_size - 1 };
      let req = DownloadRequest {
        url: url.to_string(),
        range: (offset, end),
        size: (end - offset + 1), // [offset, end]
        filename: format!("{}.part{}", filename, i)
      };

      let progress_bar = multi_progress.add(ProgressBar::new(req.size));
      progress_bar.set_style(progress_style.clone());
      progress_bar.set_message(req.filename.clone());

      handles.push(tokio::spawn(download(req, multi_progress.clone(), progress_bar)));
      offset = end + 1;
    }
   
    let res = join_all(handles).await; // .iter().map(|r| r.unwrap()? ).collect();
    let mut requests: Vec<DownloadRequest> = vec![];
    for item in res {
      let result: std::result::Result<DownloadRequest, Error> = item?;
      let request = result?; // TODO: retry
      //println!("request: {:#?}", request);
      requests.push(request);
    }
    let cost_time: f64 = start.elapsed().as_secs_f64();
    let bytes_per_sec = content_length as f64 / cost_time;

    match merge_files(requests, filename).await {
      Ok(filename) => {
        multi_progress.println(format!("download successed -> {}, speed: {}/s", filename, HumanBytes(bytes_per_sec as u64))).unwrap();
      },
      Err(e) => {
        multi_progress.println(format!("download failed")).unwrap();
      }
    }
    multi_progress.clear().unwrap();
  }

  Ok(())
}

// refs:
// https://rust-lang-nursery.github.io/rust-cookbook/web/clients/download.html
// https://github.com/console-rs/indicatif/blob/main/examples/multi.rs