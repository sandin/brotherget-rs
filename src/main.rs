extern crate log;

mod error;
mod config;
mod download;
mod ssocks;
mod p2p;
mod service;
mod event;

use std::process;
use std::path::Path;
use std::time::{Duration};
use clap::{Arg, App};
use tokio::signal;
use tokio::sync::broadcast::error::SendError;
use tokio::time::sleep;
use env_logger::{self, Env};

use crate::error::BError;
use crate::config::Config;
use crate::download::{download_file};
use crate::ssocks::{run_ssserver, run_sslocal, get_random_available_port};
use crate::p2p::{join_p2p, discover_p2p_services};
use crate::service::{P2PService, SSProxyService};
use crate::event::{Event, EventBus};

#[derive(Debug, Clone)]
struct Context {
  event_bus: EventBus,

  proxy_addr: Option<String>,
  proxy_port: Option<u32>,
  proxy_password: Option<String>,

  peer_id: Option<String>,
  peer_addr: Option<String>,
  peer_port: Option<u32>,

  url: Option<String>,
  local_proxies: Vec<String>,

  pending_exit: bool,
}

impl Default for Context {
  fn default() -> Self {
    Context {
      event_bus: EventBus::new(),

      proxy_addr: None,
      proxy_port: None,
      proxy_password: None,

      peer_id: None,
      peer_addr: None,
      peer_port: None,

      url: None,
      local_proxies: vec![],

      pending_exit: false,
    }
  }
}

impl Context {

  pub fn is_ready(&self) -> bool {
    self.proxy_addr.is_some() && self.proxy_port.is_some()
      && self.peer_id.is_some() && self.peer_addr.is_some() && self.peer_port.is_some()
  }

  pub fn post_event(&self, event: Event) -> Result<usize, SendError<Event>> {
    self.event_bus.sender.send(event)
  }

}

async fn start_server(config: Config) -> Result<(), BError> {
  let mut context = Context { 
    proxy_password: Some(config.proxy.password.clone()),
    ..Default::default()
  };

  // proxy server
  let event_bus1 = context.event_bus.clone();
  tokio::spawn(async move {
    let config_content = r#"
    {
        "server": "0.0.0.0",
        "server_port": {server_port},
        "password": "{password}",
        "method": "aes-256-gcm"
    }
    "#;
    let mut port = config.proxy.server_port;
    if port == 0 {
        port = get_random_available_port().await.unwrap();
    }
    // rust raw string literals do not support placeholder, use string replace instead
    let config_content = config_content.replace("{server_port}", &port.to_string());  
    let config_content = config_content.replace("{password}", &config.proxy.password);
    run_ssserver(Some(&config_content), event_bus1.clone()).await.unwrap();
    event_bus1.sender.send(Event::ProxyStoped).unwrap();
  });

  // p2p node
  let event_bus2 = context.event_bus.clone();
  tokio::spawn(async move {
    join_p2p(config.p2p.key_file, config.p2p.peer_port, config.p2p.bootnodes, event_bus2.clone()).await.unwrap();
    event_bus2.sender.send(Event::PeerStoped).unwrap();
  });

  fn on_ready(context: &Context) {
    let mut service: SSProxyService = SSProxyService::default();
    service.info.addr = context.peer_addr.as_ref().unwrap().clone();
    service.info.port = context.proxy_port.unwrap() as i32;
    service.info.password = context.proxy_password.as_ref().unwrap().clone();

    let service_key = format!("{}_{}", service.service_name(), context.peer_id.as_ref().unwrap());
    context.post_event(Event::PutRecord { key: service_key, value: service.serialize() }).unwrap();
    context.post_event(Event::StartProviding { key: SSProxyService::default().service_name() }).unwrap();
    context.post_event(Event::GetProviders { key: SSProxyService::default().service_name() }).unwrap();
  }

  fn on_exit(context: &mut Context) {
    let service_key = format!("{}_{}", SSProxyService::default().service_name(), context.peer_id.as_ref().unwrap());
    context.post_event(Event::PutRecord { key: service_key, value: vec![] }).unwrap(); // TODO: wait for it
    context.pending_exit = true; // graceful exit
  }

  // STATE MACHINE:
  //         Idle 
  //           |   start_ssserver()         // start proxy server
  //           v
  //      ProxyStarted                      
  //           |   join_p2p()               // join p2p network
  //           v
  //      PeerStarted                       
  //           |   post_event(GetProviders) // find p2p peers
  //           v
  //   GetProvidersResult
  //           |   post_event(GetRecord)    // find proxy url of each peer(for debug)
  //           v
  //    GetRecordResult
  //           |   println!(proxy_url)      // just print proxies url(for debug)
  //           v
  //  PeerStoped/ProxyStoped
  //
  tokio::pin!(context);
  loop {
    tokio::select! {
      e = context.event_bus.receiver.recv() => {
        match e.unwrap() {
          Event::PeerStarted { peer_id, addr, port } => {
            println!("peer is ready, addr={}, port={}", addr, port);
            println!("press Ctrl+C to stop");
            context.peer_id = Some(peer_id);
            context.peer_addr = Some(addr);
            context.peer_port = Some(port);
            if context.is_ready() {
              on_ready(&context);
            }
          },
          Event::ProxyStarted { addr, port } => {
            println!("proxy is ready, addr={}, port={}", addr, port);
            context.proxy_addr = Some(addr);
            context.proxy_port = Some(port);
            if context.is_ready() {
              on_ready(&context);
            }
          },
          Event::PeerStoped | Event::ProxyStoped => {
            println!("peer/proxy stoped");
            on_exit(&mut context);
          },
          Event::PutRecordResult { success } => {
            if context.pending_exit {
              println!("graceful exit");
              process::exit(if success { 0 } else { -1 });
            }
          },
          Event::GetProvidersResult { key, providers } => {
            println!("found providers(key={}): {:#?}", &key, providers);
            for provider in providers {
              context.post_event(Event::GetRecord { key: provider.clone() }).unwrap();
            }
          },
          Event::GetRecordResult { key, value } => {
            println!("found record: peer_id={}, service={:#?}", &key, value);
          },
          _ => {}
        }
      },
      _ = signal::ctrl_c() => {
        println!("exit by canceled");
        on_exit(&mut context);
      },
    }
  }
}

async fn download_url(url: String, config: Config) -> Result<(), BError> {
  println!("download file, url={}", url);
  let context = Context {
    url: Some(url.clone()),
    ..Default::default()
  };

  // finding peers that provide proxy services on a private P2P network,
  // these brothers will share their bandwidth to speed up our downloads.
  let timeout = Duration::from_secs(5 * 60);
  let remote_proxy_services: Vec<SSProxyService> = discover_p2p_services(SSProxyService::default().service_name(), config.p2p.key_file, config.p2p.peer_port, config.p2p.bootnodes, timeout).await.unwrap();
  let remote_proxy_services = remote_proxy_services.into_iter().filter(|s| s.info.addr.len() > 0 && s.info.port != 0).collect::<Vec<SSProxyService>>();
  println!("found proxy services: {:#?}", remote_proxy_services);

  // use `sslocal` to connect to each proxy servers that use `ssserver`
  // running on the brother peers, and run http/socks5 proxy locally.
  let mut local_proxy_handles = vec![];
  for proxy_service in remote_proxy_services.iter() {
    let mut sslocal_config = config.proxy.clone();
    sslocal_config.server = proxy_service.info.addr.clone();
    sslocal_config.server_port = proxy_service.info.port as u32;
    sslocal_config.password = proxy_service.info.password.clone();
    let event_bus = context.event_bus.clone();
    local_proxy_handles.push(tokio::spawn(async move {
      let config_content = r#"
      {
        "server": "{server}",
        "server_port": {server_port},
        "password": "{password}",
        "method": "aes-256-gcm",
        "protocol": "http",
        "local_address": "127.0.0.1",
        "local_port": {local_port}
      }
      "#;
      let mut local_port = sslocal_config.local_port;
      if local_port == 0 {
        local_port = get_random_available_port().await.unwrap();
      }
      // rust raw string literals do not support placeholder, use string replace instead
      let config_content = config_content.replace("{server}", &sslocal_config.server.to_string());  
      let config_content = config_content.replace("{server_port}", &sslocal_config.server_port.to_string());  
      let config_content = config_content.replace("{password}", &sslocal_config.password);
      let config_content = config_content.replace("{local_port}", &local_port.to_string());
      println!("run local proxy: {}", &config_content);

      run_sslocal(Some(&config_content), event_bus.clone()).await.unwrap();
      event_bus.sender.send(Event::ProxyStoped).unwrap();
    }));
  }

  fn on_ready(context: &Context) {
    let url = context.url.as_ref().unwrap().clone();
    let proxies = context.local_proxies.clone();
    let event_bus = context.event_bus.clone();
    tokio::spawn(async move {
      match download_file(url, proxies).await {
        Ok(filename) => event_bus.sender.send(Event::DownloadSuccessed { filename }).unwrap(),
        Err(err) => event_bus.sender.send(Event::DownloadFailed { error: err.to_string() }).unwrap(),
      }
    });
  }

  fn on_exit(_context: &Context) {
    // unfortunately we can not control any local proxy server to stop or cannel any outbounding http request,
    // we just exit and everything will shutdown immediately
    process::exit(0);
  }

  let timeout_f = sleep(timeout); // future
  tokio::pin!(timeout_f);
  tokio::pin!(context);
  loop {
    tokio::select! {
      e = context.event_bus.receiver.recv() => {
        match e.unwrap() {
          Event::ProxyStarted { addr, port } => {
            println!("proxy is ready, addr={}, port={}", addr, port);
            context.local_proxies.push(format!("{}:{}", addr, port));
            if context.local_proxies.len() == remote_proxy_services.len() {
              on_ready(&context);
            }
          },
          Event::ProxyStoped => {
            println!("proxy stoped");
            on_exit(&context);
          },
          Event::DownloadSuccessed { filename } => {
            println!("download success, output filename: {}", filename);
            on_exit(&context);
          },
          Event::DownloadFailed { error } => {
            println!("download failed, error: {:#?}", error);
            on_exit(&context);
          },
          _ => {}
        }
      },
      _ = &mut timeout_f => {
        eprintln!("find proxies timeout");
        on_ready(&context); // download directly without any proxy
      },
      _ = signal::ctrl_c() => {
        println!("exit by canceled");
        on_exit(&context);
      },
    }
  }
}

#[tokio::main]
async fn main() -> Result<(), BError> {
  let matches  = App::new("bget")
    .version(env!("CARGO_PKG_VERSION"))
    .about("BrotherGet is a P2P downloader.")
    .arg(
      Arg::with_name("config")
          .short("c")
          .long("config")
          .takes_value(true)
          .help("config json file"),
    )
    .arg(
      Arg::with_name("verbose")
          .short("v")
          .long("verbose")
          .takes_value(false)
          .help("enable debug log"),
    )
    .arg(
      Arg::with_name("url").help("url")
    )
    .get_matches();

  if matches.is_present("verbose") {
    env_logger::Builder::from_env(Env::default().default_filter_or("debug")).init();
  } else {
    env_logger::init();
  }

  let config: Config = match matches.value_of("config") {
    Some(filename) => Config::load_from_file(filename).await.expect(&format!("can not parse config file {}", filename)),
    None => Config::default()
  };

  match matches.value_of("url") {
    Some(url) => download_url(String::from(url), config).await?,
    None => start_server(config).await?,
  };

  Ok(())
}