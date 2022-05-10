use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, Condvar};
use std::path::{PathBuf};
use std::env;
use std::thread;

use crate::error::BError;
use crate::config::{Config};

/*
#[cfg(target_os = "windows")]
const SSLOCAL_BIN: &str = "sslocal.exe";
#[cfg(target_os = "macos")]
const SSLOCAL_BIN: &str = "sslocal";
#[cfg(target_os = "linux")]
const SSLOCAL_BIN: &str = "sslocal";

pub fn setup_local_proxy(server: Server, stop_cond: Arc<(Mutex<bool>, Condvar)>) -> Result<Option<String>, BError> {
  if !server.local_address.is_empty() && server.local_port != 0 {
    // proxy url format: "http://127.0.0.1:1080" or "socks5://127.0.0.1:1080"
    let proxy_url = format!("{}://{}:{}", &server.protocol, &server.local_address, server.local_port); 

    let mut sslocal_path = PathBuf::new();
    sslocal_path.push(env::current_dir()?);
    sslocal_path.push(SSLOCAL_BIN);
    if !sslocal_path.exists() {
      return Err(BError::Proxy(format!("`{}` file is not exists", sslocal_path.display())));
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
*/


pub async fn start_proxy_server(config: &Config) -> Result<(), BError> {
  // TODO
  Ok(())
}