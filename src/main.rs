use std::io::SeekFrom;
use std::time::{Instant};
use std::path::{Path, PathBuf};
use std::process;
use std::env;
use std::thread;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, Condvar};
use error_chain::error_chain;
use futures::future::join_all;
use futures::StreamExt;
use reqwest::header::{ACCEPT_RANGES, CONTENT_LENGTH, RANGE};
use reqwest::Response;
use reqwest::StatusCode;
use tokio::fs::{remove_file, File};
use tokio::io::{AsyncSeekExt, AsyncWriteExt, AsyncReadExt};
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
  proxy: Option<String>,
  server: Server,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Server {
  server: String,
  server_port: i32,
  password: String,
  method: String,
  protocol: String,
  local_address: String,
  local_port: i32,
}

#[derive(Serialize, Deserialize, Debug)]
struct Config {
  servers: Vec<Server>
}

async fn download(request: DownloadRequest, _: MultiProgress, progress_bar: ProgressBar) -> Result<DownloadRequest> {
  let mut output_file = File::create(&request.filename).await?;

  let mut builder = reqwest::Client::builder();
  let agent_name; 
  if let Some(ref proxy_url) = request.proxy {
    let proxy = reqwest::Proxy::all(proxy_url)?;
    builder = builder.proxy(proxy);
    agent_name = format!("{}:{}", &request.server.server, request.server.server_port);
  } else {
    agent_name = "localhost".to_string();
  }
  progress_bar.set_message(format!("{} {}-{}", agent_name, request.range.0, request.range.1));

  let client = builder.build()?; 
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

async fn parse_config_file(config_filename: &str) -> Result<Config> {
  if !Path::new(config_filename).to_path_buf().exists() {
    error_chain::bail!("Error: config file is missing.")
  }

  // parse config file
  let mut config_file = File::open(config_filename).await?;
  let mut buffer = Vec::new();
  let n: usize = config_file.read_to_end(&mut buffer).await?;
  let mut config: Config = match serde_json::from_slice(&buffer[0..n]) {
    Ok(r) => r,
    Err(_e) => {
      error_chain::bail!(format!("Error: can not parse config file {}.", config_filename));
    }
  };
  config.servers.insert(0, Server {
    server: "127.0.0.1".to_string(),
    server_port: 0,
    password: "".to_string(),
    method: "".to_string(),
    protocol: "http".to_string(),
    local_address: "127.0.0.1".to_string(),
    local_port: 0,
  });

  Ok(config)
}

#[cfg(target_os = "windows")]
const SSLOCAL_BIN: &str = "sslocal.exe";
#[cfg(target_os = "macos")]
const SSLOCAL_BIN: &str = "sslocal";
#[cfg(target_os = "linux")]
const SSLOCAL_BIN: &str = "sslocal";

fn setup_local_proxy(server: Server, stop_cond: Arc<(Mutex<bool>, Condvar)>) -> Result<Option<String>> {
  if !server.local_address.is_empty() && server.local_port != 0 {
    // proxy url format: "http://127.0.0.1:1080" or "socks5://127.0.0.1:1080"
    let proxy_url = format!("{}://{}:{}", &server.protocol, &server.local_address, server.local_port); 

    let mut sslocal_path = PathBuf::new();
    sslocal_path.push(env::current_dir()?);
    sslocal_path.push(SSLOCAL_BIN);
    if !sslocal_path.exists() {
      error_chain::bail!(format!("`{}` file is not exists", sslocal_path.display()));
    }

    thread::spawn(move || {
      let mut child = Command::new(&sslocal_path)
        .arg("--local-addr").arg(format!("{}:{}", &server.local_address, server.local_port)) // e.g.: --local-addr=127.0.0.1:1081
        .arg("--server-addr").arg(format!("{}:{}", &server.server, server.server_port)) // e.g.: --server-addr=10.11.246.46:8383
        .arg("--password").arg(server.password.clone()) // e.g.: --password="mMVlD2/6lni6EX6l5Tx3khJcl7Y="
        .arg("--encrypt-method").arg(server.method.clone()) // e.g.: --encrypt-method="aes-256-gcm"
        .arg("--protocol").arg(server.protocol.clone()) // e.g.: --protocol=http
        .stdout(Stdio::null()) // keep silent
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn");
      //let status = child.wait();
      //println!("the command exited with: {}", status);

      // let's wait for master's call
      let (lock, cvar) = &*stop_cond;
      let _guard = cvar.wait_while(lock.lock().unwrap(), |stop| { !*stop }).unwrap();

      // master told us to kill myself, just do it.
      child.kill().unwrap(); 
      println!("stop local proxy server at {}:{}", &server.local_address, server.local_port);
    });
    Ok(Some(proxy_url))
  } else {
    Ok(None) // It's OK, not an error, just DO NOT use any proxy, e.g.: localhost
  }
}

async fn start_server(config: &Config) -> Result<()> {


  Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
  let matches  = App::new("bget")
    .version(env!("CARGO_PKG_VERSION"))
    .arg(
      Arg::with_name("config")
          .long("config")
          .takes_value(false)
          .help("config json file"),
    )
    .arg(
      Arg::with_name("url").required(true).help("target url")
    )
    .get_matches();

  let url = matches.value_of("url").unwrap();
  let filename = match url.rfind("/") {
    Some(i) => &url[(i+1)..], // url = url.substring(url.lastIndexOf("/"))
    None => url
  };

  // Parse config file
  let config_filename = matches.value_of("config").unwrap_or("config.json");
  let config: Config = match parse_config_file(config_filename).await {
    Ok(r) => r,
    Err(_e) => {
      eprintln!("Error: can not parse config file {}.", config_filename);
      process::exit(-1);
    }
  };

  if url == "start_server" {
    return start_server(&config).await;
  }

  // Send HEAD request for range
  let (support_range, content_length) = head(url).await?;
  let chunk_count: u32 = config.servers.len() as u32;
  let mut handles = vec![];
  let stop_cond = Arc::new((Mutex::new(false), Condvar::new()));
  if support_range {
    let chunk_size: u64 = (content_length as f64 / chunk_count as f64).ceil() as u64;
    let last_chunk_size: u64 = if content_length % chunk_size == 0 { chunk_size } else { content_length % chunk_size };
    let total_content_length = (chunk_size * (chunk_count - 1) as u64) + last_chunk_size;
    println!(
      "url: {}, content_length: {}, chunk_count: {}, chunk_length: {}, last_chunk_length: {}",
      url, content_length, chunk_count, chunk_size, last_chunk_size
    );
    assert!(total_content_length == content_length);

    let multi_progress = MultiProgress::new();
    let progress_style = ProgressStyle::with_template("[eta {eta_precise}] {bar:40.cyan/blue} {percent:>3}% {binary_bytes_per_sec} {msg}")
      .unwrap()
      .progress_chars("##-");

    // Send GET requests for file
    multi_progress.println("start downloading...").unwrap();
    let start = Instant::now();
    let mut offset = 0;
    let mut i = 0;
    for server in config.servers {
      let end = if i != chunk_count - 1 { offset + chunk_size - 1 } else { offset + last_chunk_size - 1 };
      let proxy_url = match setup_local_proxy(server.clone(), Arc::clone(&stop_cond)) {
        Ok(r) => { 
          if !r.is_none() {
            multi_progress.println(format!("start local proxy server at {}:{} => {}:{}",
              &server.local_address, server.local_port, &server.server, server.server_port)).unwrap();
          }
          r
        },     // local proxy server are ready to go
        Err(_e) => None // just ignore the error and do not use any proxy
      };

      let req = DownloadRequest {
        url: url.to_string(),
        range: (offset, end),
        size: (end - offset + 1), // bytes range = [offset, end]
        filename: format!("{}.part{}", filename, i),
        proxy: proxy_url,
        server: server,
      };

      let progress_bar = multi_progress.add(ProgressBar::new(req.size));
      progress_bar.set_style(progress_style.clone());
      progress_bar.set_message(req.filename.clone());

      handles.push(tokio::spawn(download(req, multi_progress.clone(), progress_bar)));
      offset = end + 1;
      i += 1;
    }
   
    // Let's wait
    let res = join_all(handles).await;
    let mut responses: Vec<DownloadRequest> = vec![]; // typedef DownloadRequest DownloadResponse
    for item in res {
      let result: std::result::Result<DownloadRequest, Error> = item?;
      responses.push(result?);
    }
    let cost_time: f64 = start.elapsed().as_secs_f64();
    let bytes_per_sec = content_length as f64 / cost_time;

    // notify all sslocal subprocesses to stop themselves
    let (lock, cvar) = &*stop_cond;
    let mut stop = lock.lock().unwrap();
    *stop = true;
    cvar.notify_all();

    // Merge
    multi_progress.println("start merge files...").unwrap();
    match merge_files(responses, filename).await {
      Ok(filename) => {
        multi_progress.println(format!("download successed -> {}, speed: {}/s", filename, HumanBytes(bytes_per_sec as u64))).unwrap();
      },
      Err(_e) => {
        multi_progress.println(format!("download failed")).unwrap();
      }
    }
    multi_progress.clear().unwrap();
  } else {
    // TODO: Download directly 
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