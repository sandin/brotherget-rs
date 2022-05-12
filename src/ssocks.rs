use std::error::Error;
use clap::{Arg, App};
use tokio::net::TcpListener;
use shadowsocks_service::{run_server, create_local, config as SSConfig};
use crate::event::{EventBus, Event};

pub async fn get_random_available_port() -> Result<u32, Box<dyn Error>> {
    let listener = TcpListener::bind("0.0.0.0:0").await?;
    let port = listener.local_addr()?.port() as u32;
    Ok(port)
}

pub async fn start_ssserver(config_str: Option<&str>, event_bus: EventBus) -> Result<(), Box<dyn Error>> {
    println!("start_ssserver");

    let config: SSConfig::Config = match config_str {
        Some(content) => SSConfig::Config::load_from_str(content, SSConfig::ConfigType::Server).unwrap(),
        None => { 
            let config_content = r#"
                {
                    "server": "0.0.0.0",
                    "server_port": 8383,
                    "password": "mMVlD2/6lni6EX6l5Tx3khJcl7Y=",
                    "method": "aes-256-gcm"
                }
            "#;
            SSConfig::Config::load_from_str(config_content, SSConfig::ConfigType::Server).unwrap()
         },
    };

    println!("proxy server listen at {:#?}", config.server[0].addr());
    event_bus.sender.send(Event::ProxyStarted { 
        addr: config.server[0].addr().host(),
        port: config.server[0].addr().port() as u32, 
    }).unwrap();
    run_server(config).await?;
    Ok(())
}

pub async fn start_sslocal(config_str: Option<&str>, event_bus: EventBus) -> Result<(), Box<dyn Error>> {
    println!("start_sslocal");

    let config: SSConfig::Config = match config_str {
        Some(content) => SSConfig::Config::load_from_str(content, SSConfig::ConfigType::Local).unwrap(),
        None => { 
            let config_content = r#"
                {
                    "server": "127.0.0.1",
                    "server_port": 8383,
                    "password": "mMVlD2/6lni6EX6l5Tx3khJcl7Y=",
                    "method": "aes-256-gcm",
                    "protocol": "http",
                    "local_address": "127.0.0.1",
                    "local_port": 1093
                }
            "#;
            SSConfig::Config::load_from_str(config_content, SSConfig::ConfigType::Local).unwrap()
         },
    };

    println!("local proxy protocol: {:#?} addr: {:#?}", config.local[0].protocol, config.local[0].addr);
    let addr = config.local[0].addr.as_ref().unwrap();
    event_bus.sender.send(Event::ProxyStarted { 
        addr: addr.host(),
        port: addr.port() as u32, 
    }).unwrap();
    let instance = create_local(config).await?;
    instance.wait_until_exit().await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {

    let matches  = App::new("bsss")
    .version(env!("CARGO_PKG_VERSION"))
    .arg(
      Arg::with_name("config")
          .short("c")
          .long("config")
          .takes_value(true)
          .help("config file"),
    )
    .arg(
      Arg::with_name("mode").required(true).help("server/client mode")
    )
    .get_matches();

    let event_bus = EventBus::new();
    match matches.value_of("mode") {
        Some("server") => {
            start_ssserver(matches.value_of("config"), event_bus.clone()).await?;
        },
        Some("client") => {
            start_sslocal(matches.value_of("config"), event_bus.clone()).await?;
        },
        Some(&_) | None => {
            eprintln!("unknown mode");
        },
    };

    Ok(())
}