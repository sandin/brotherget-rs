use std::path::{Path};
use std::default::Default;
use serde::{Serialize, Deserialize}; 
use tokio::fs::{File};
use tokio::io::{AsyncReadExt};
use crate::error::BError;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Server {
  pub server: String,
  pub server_port: i32,
  pub password: String,
  pub method: String,
  pub protocol: String,
  pub local_address: String,
  pub local_port: i32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
  pub servers: Vec<Server>, // TODO: change to  bootnodes

  pub server_port: u32, // port of ssserver
  pub password: String, // password of sserver and sslocal

  pub peer_port: u32, // port of p2p
  pub key_file: Option<String>, // keyfile of p2p
  pub bootnodes: Vec<String>, // bootnodes of p2p
}

impl Default for Config {
  fn default() -> Self {
    Config {
      servers: vec![],
      server_port: 8383, 
      password: String::from("foo!bar!"),
      peer_port: 53308,
      key_file: None,
      bootnodes: vec![],
    }
  }
}

impl Config {

  pub async fn load_from_file(filename: &str) -> Result<Config, BError> {
    if !Path::new(filename).to_path_buf().exists() {
      return Err(BError::Config(format!("Error: config file is missing.")));
    }
  
    // parse config file
    let mut config_file = File::open(filename).await?;
    let mut buffer = Vec::new();
    let n: usize = config_file.read_to_end(&mut buffer).await?;
    let mut config: Config = match serde_json::from_slice(&buffer[0..n]) {
      Ok(r) => r,
      Err(_e) => {
        return Err(BError::Config(format!("Error: can not parse config file {}.", filename)));
      }
    };

    // TODO: delete me
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

}