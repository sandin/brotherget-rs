use std::path::{Path};
use std::default::Default;
use serde::{Serialize, Deserialize}; 
use tokio::fs::{File};
use tokio::io::{AsyncReadExt};
use crate::error::BError;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProxyConfig {
  pub local_port: u32,  // port of sslocal
  pub server: String,   // addr of ssserver
  pub server_port: u32, // port of ssserver
  pub password: String, // password of sserver and sslocal
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct P2pConfig {
  pub peer_port: u32, // port of p2p
  pub key_file: Option<String>, // keyfile of p2p
  pub bootnodes: Vec<String>, // bootnodes of p2p
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
  pub proxy: ProxyConfig,
  pub p2p: P2pConfig,
}

impl Default for Config {
  fn default() -> Self {
    Config {
      proxy: ProxyConfig {
        local_port: 0,
        server: String::from("0.0.0.0"),
        server_port: 0, 
        password: String::from("mMVlD2/6lni6EX6l5Tx3khJcl7Y="),
      },
      p2p: P2pConfig {
        peer_port: 0,
        key_file: None,
        bootnodes: vec![
          String::from("/ip4/10.11.64.68/tcp/53308/p2p/QmVN7pykS5HgjHSGS3TSWdGqmdBkhsSj1G5XLrTconUUxa"),
          String::from("/ip4/10.11.251.117/tcp/53309/p2p/QmckweETKF3ncFQRYYwbbBHZRH6RhqPkG3sCjF5E68DkP9")
        ],
      }
    }
  }
}

impl Config {

  pub async fn load_from_file(filename: &str) -> Result<Config, BError> {
    if !Path::new(filename).exists() {
      return Err(BError::Config(format!("Error: config file is missing.")));
    }
  
    // parse config file
    let mut config_file = File::open(filename).await?;
    let mut buffer = Vec::new();
    let n: usize = config_file.read_to_end(&mut buffer).await?;
    let config: Config = match serde_json::from_slice(&buffer[0..n]) {
      Ok(r) => r,
      Err(_e) => {
        return Err(BError::Config(format!("Error: can not parse config file {}.", filename)));
      }
    };

    Ok(config)
  }

  pub fn is_bootnode(&self) -> bool {
    self.p2p.key_file.is_some()
  }
}