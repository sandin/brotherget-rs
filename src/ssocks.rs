#[macro_use] extern crate log;
extern crate env_logger;
use std::error::Error;
use clap::{Arg, App};
use shadowsocks_service::{run_server, create_local, config as SSConfig};

async fn start_ssserver(config_file: Option<&str>) -> Result<(), Box<dyn Error>> {
    println!("start_ssserver");

    let config: SSConfig::Config = match config_file {
        Some(filename) => SSConfig::Config::load_from_file(filename, SSConfig::ConfigType::Server).unwrap(),
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
    run_server(config).await?;
    Ok(())
}

async fn start_sslocal(config_file: Option<&str>) -> Result<(), Box<dyn Error>> {
    println!("start_sslocal");

    let config: SSConfig::Config = match config_file {
        Some(filename) => SSConfig::Config::load_from_file(filename, SSConfig::ConfigType::Local).unwrap(),
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
    let instance = create_local(config).await?;
    instance.wait_until_exit().await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

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

    match matches.value_of("mode") {
        Some("server") => {
            start_ssserver(matches.value_of("config")).await?;
        },
        Some("client") => {
            start_sslocal(matches.value_of("config")).await?;
        },
        Some(&_) | None => {
            eprintln!("unknown mode");
        },
    };

    Ok(())
}