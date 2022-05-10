use std::time::{Instant};
use std::process;
use std::sync::{Arc, Mutex, Condvar};
use futures::future::join_all;
use futures::future::select_all;
use futures::future::{self, Either};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle, HumanBytes};
use clap::{Arg, App, SubCommand};
use tokio::signal;
use tokio::sync::mpsc;

mod error;
mod proxy;
mod config;
mod download;
mod ssocks;
mod p2p;
mod event;

use crate::error::BError;
use crate::proxy::{start_proxy_server};
use crate::config::Config;
use crate::download::{DownloadRequest, head, download, merge_files};
use crate::ssocks::{start_ssserver, start_sslocal};
use crate::p2p::{join_p2p};
use crate::event::{Event, EventBus};

async fn start_server(config: Config) -> Result<(), BError> {
  let event_bus = EventBus::new();

  // proxy server
  let proxy_handle = tokio::spawn(async move {
    let config_content = r#"
    {
        "server": "0.0.0.0",
        "server_port": {server_port},
        "password": "{password}",
        "method": "aes-256-gcm"
    }
    "#;
    // rust raw string literals do not support placeholder, use string replace instead
    let config_content = config_content.replace("{server_port}", &config.proxy.server_port.to_string());  
    let config_content = config_content.replace("{password}", &config.proxy.password);
    return start_ssserver(Some(&config_content)).await.or(Err(BError::Other(format!("Can not start proxy server"))));
  });

  // p2p node
  let event_bus1 = event_bus.clone();
  let p2p_handle = tokio::spawn(async move {
    return join_p2p(config.p2p.key_file, config.p2p.peer_port, config.p2p.bootnodes, event_bus1).await.or(Err(BError::Other(format!("Can not start proxy server"))));
  });
  
  println!("press Ctrl+C to stop");
  let handles = vec![proxy_handle, p2p_handle];
  //tokio::pin!(handles);

  let mut rx = event_bus.receiver;
  let tx = event_bus.sender;
  loop {
    tokio::select! {
      e = rx.recv() => {
        match e.unwrap() {
          Event::PeerReady => {

          },
          _ => {}
        }
      },
      _ = select_all(handles) => {
          println!("handles select all end");
          process::exit(0);
      },
      _ = signal::ctrl_c() => {
        println!("exit by canceled");
        process::exit(0);
      },
    }
  }
  Ok(())
}

async fn download_file(url: &str, config: Config) -> Result<(), BError> {
  println!("download_file, url={}", url);

  Ok(())
}


#[tokio::main]
async fn main() -> Result<(), BError> {
  let matches  = App::new("bget")
    .version(env!("CARGO_PKG_VERSION"))
    .about("BrotherGet is a p2p downloader.")
    .arg(
      Arg::with_name("config")
          .long("config")
          .takes_value(true)
          .help("config json file"),
    )
    .arg(
      Arg::with_name("url").help("url")
    )
    .get_matches();

  let config: Config = match matches.value_of("config") {
    Some(filename) => Config::load_from_file(filename).await.expect(&format!("can not parse config file {}", filename)),
    None => Config::default(),
  };

  match matches.value_of("url") {
    Some(url) => download_file(url, config).await?,
    None => start_server(config).await?,
  };

  Ok(())
}


/*

async fn main1() -> Result<(), BError> {
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
    return start_proxy_server(&config).await;
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
      let result: std::result::Result<DownloadRequest, BError> = item?;
      match result {
        Ok(r) => responses.push(r),
        Err(e) => println!("{:#?}", e),
      }
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

*/

// refs:
// https://rust-lang-nursery.github.io/rust-cookbook/web/clients/download.html
// https://github.com/console-rs/indicatif/blob/main/examples/multi.rs

// TODO:
// test
//let url = "https://releases.ubuntu.com/22.04/ubuntu-22.04-desktop-amd64.iso";
//let url = "https://dl.google.com/go/go1.18.1.windows-amd64.msi"; // 132MB SHA256 Checksum: 9a3e9636eb5f9af4b3b29b116b7d28b8d6092c690ee4b88cfb9613b8851d4a66
//let url = "https://golang.google.cn/dl/go1.18.1.src.tar.gz"; // 22MB SHA256 Checksum: efd43e0f1402e083b73a03d444b7b6576bb4c539ac46208b63a916b69aca4088
