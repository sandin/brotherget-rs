use std::io::SeekFrom;
use std::time::{Instant};
use indicatif::{MultiProgress, ProgressStyle, ProgressBar, HumanBytes};
use futures::future::join_all;
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
  pub range: Option<(u64, u64)>,
  pub size: u64,
  pub filename: String,
  pub proxy_url: Option<String>,
  pub proxy_name: String,
}

pub async fn download_file(url: String, proxies: Vec<String>) -> Result<String, BError> {
  let multi_progress = MultiProgress::new();
  let progress_style = ProgressStyle::with_template("[eta {eta_precise}] {bar:40.cyan/blue} {percent:>3}% {binary_bytes_per_sec} {msg}")
    .unwrap()
    .progress_chars("##-");

  let filename = url.clone();
  let filename = match filename.rfind("/") {
    Some(i) => String::from(&filename[(i+1)..]), // url = url.substring(url.lastIndexOf("/"))
    None => filename
  };

  // Send HEAD request for range
  let mut requestes: Vec<DownloadRequest> = vec![];
  let (support_range, content_length) = head(url.clone()).await?;
  if support_range {
    let chunk_count: u32 = proxies.len() as u32;
    let chunk_size: u64 = (content_length as f64 / chunk_count as f64).ceil() as u64;
    let last_chunk_size: u64 = if content_length % chunk_size == 0 { chunk_size } else { content_length % chunk_size };
    let total_content_length = (chunk_size * (chunk_count - 1) as u64) + last_chunk_size;
    println!(
      "url: {}, content_length: {}, chunk_count: {}, chunk_length: {}, last_chunk_length: {}",
      url, content_length, chunk_count, chunk_size, last_chunk_size
    );
    assert!(total_content_length == content_length);

    let mut offset = 0;
    let mut i = 0;
    let end = if i != chunk_count - 1 { offset + chunk_size - 1 } else { offset + last_chunk_size - 1 };
    for proxy in proxies {
      requestes.push(DownloadRequest {
        url: url.to_string(),
        range: Some((offset, end)),
        size: (end - offset + 1), // bytes range = [offset, end]
        filename: format!("{}.part{}", filename, i),
        proxy_url: Some(proxy.clone()),
        proxy_name: proxy, // TODO
      });

      offset = end + 1;
      i += 1;
    }
  } else {
    // Download directly 
    requestes.push(DownloadRequest {
      url: url.to_string(),
      range: None,
      size: 0,
      filename: filename.clone(),
      proxy_url: None,
      proxy_name: String::from("localhost"),
    });
  }

  let mut handles = vec![];

  // Send GET requests for file
  multi_progress.println("start downloading...").unwrap();
  let start = Instant::now();
 
  for request in requestes {
    let progress_bar = multi_progress.add(ProgressBar::new(request.size));
    progress_bar.set_style(progress_style.clone());
    progress_bar.set_message(request.filename.clone());

    handles.push(tokio::spawn(download_partial(request, multi_progress.clone(), progress_bar)));
  }
  
  // Let's wait
  let res = join_all(handles).await;
  let mut responses: Vec<DownloadRequest> = vec![]; // typedef DownloadRequest DownloadResponse
  for item in res {
    let result: std::result::Result<DownloadRequest, BError> = item?;
    match result {
      Ok(r) => responses.push(r),
      Err(e) => println!("{:#?}", e),
    }
  }
  let cost_time: f64 = start.elapsed().as_secs_f64();
  let bytes_per_sec = content_length as f64 / cost_time;

  // Merge
  multi_progress.println("start merge files...").unwrap();
  match merge_files(responses, filename.clone()).await {
    Ok(()) => {
      multi_progress.println(format!("download successed -> {}, speed: {}/s", &filename, HumanBytes(bytes_per_sec as u64))).unwrap();
    },
    Err(_e) => {
      multi_progress.println(format!("download failed")).unwrap();
    }
  }
  multi_progress.clear().unwrap();
  
  Ok(filename)
}

pub async fn download_partial(request: DownloadRequest, _: MultiProgress, progress_bar: ProgressBar) -> Result<DownloadRequest, BError> {
  let mut output_file = File::create(&request.filename).await?;

  let builder = reqwest::Client::builder();
  let mut agent_name = String::from("localhost");
  let client = match request.proxy_url {
    Some(ref proxy_url) => {
      agent_name = format!("{}", &request.proxy_name);
      let proxy = reqwest::Proxy::all(proxy_url)?;
      builder.proxy(proxy).build()?
    },
    None => {
      builder.build()?
    }
  };

  let mut support_range = false;
  let request_builder = match request.range {
    Some(range) => {
      support_range = true;
      progress_bar.set_message(format!("{} {}-{}", agent_name, range.0, range.1));
      client.get(&request.url).header(RANGE, format!("bytes={}-{}", range.0, range.1))
    },
    None => {
      progress_bar.set_message(format!("{}", agent_name));
      client.get(&request.url)
    }
  };

  let response: Response = request_builder.send().await?;
  let status = response.status();
  if status != StatusCode::OK || (support_range && status != StatusCode::PARTIAL_CONTENT) {
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

pub async fn head(url: String) -> Result<(bool, u64), BError> {
  let client = reqwest::Client::builder().build()?;
  let resp = client.head(url).send().await?;
  let headers = resp.headers();
  //println!("url: {}, headers: {:#?}", url, headers);

  let support_range: bool = headers.contains_key(ACCEPT_RANGES);
  let content_length: u64 = headers.get(CONTENT_LENGTH).unwrap().to_str()?.parse().unwrap(); // TODO: match

  Ok((support_range, content_length))
}

pub async fn merge_files(requests: Vec<DownloadRequest>, output_filename: String) -> Result<(), BError> {
  let mut file = File::create(output_filename).await?;

  let mut requests = requests;
  requests.sort_by(|a, b| a.filename.cmp(&b.filename));
  for request in requests {
    let mut part_file = File::open(&request.filename).await?;
    let mut buffer = Vec::new();
    let n: usize = part_file.read_to_end(&mut buffer).await?;  // TODO: stream read, buffer write
    let offset = match request.range {
      Some(range) => range.0,
      None => 0,
    };
    file.seek(SeekFrom::Start(offset)).await?; 
    //println!("copy file {} -> {} at {}, bytes {}", &request.filename, &output_filename, request.range.0, n);
    file.write_all(&buffer[..n]).await?;

    remove_file(&request.filename).await?;
  }

  Ok(())
}


// refs:
// https://rust-lang-nursery.github.io/rust-cookbook/web/clients/download.html
// https://github.com/console-rs/indicatif/blob/main/examples/multi.rs

// TODO:
// test
//let url = "https://releases.ubuntu.com/22.04/ubuntu-22.04-desktop-amd64.iso";
//let url = "https://dl.google.com/go/go1.18.1.windows-amd64.msi"; // 132MB SHA256 Checksum: 9a3e9636eb5f9af4b3b29b116b7d28b8d6092c690ee4b88cfb9613b8851d4a66
//let url = "https://golang.google.cn/dl/go1.18.1.src.tar.gz"; // 22MB SHA256 Checksum: efd43e0f1402e083b73a03d444b7b6576bb4c539ac46208b63a916b69aca4088
