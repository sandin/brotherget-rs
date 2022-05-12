use std::error::Error;
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
        None => panic!("missing config arg")
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
        None => panic!("missing config arg")
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

#[tokio::test]
async fn test_get_random_available_port() {
    let port = get_random_available_port().await.unwrap();
    println!("port {}", port);
    assert!(port > 0);
}

// cargo test -- --nocapture test_start_ssserver
#[tokio::test(start_paused = true)]
async fn test_start_ssserver() {
    let event_bus = EventBus::new();
    let config = r#"
        {
            "server": "0.0.0.0",
            "server_port": 8383,
            "password": "mMVlD2/6lni6EX6l5Tx3khJcl7Y=",
            "method": "aes-256-gcm"
        }
    "#;
    start_ssserver(Some(config), event_bus.clone()).await.unwrap();
}

// cargo test -- --nocapture test_start_sslocal
#[tokio::test(start_paused = true)]
async fn test_start_sslocal() {
    let event_bus = EventBus::new();
    let config = r#"
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
    start_sslocal(Some(config), event_bus.clone()).await.unwrap();
}