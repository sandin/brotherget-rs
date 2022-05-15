use prost::Message;
use crate::error::BError;

// generated from *.proto.
pub mod service {
    include!(concat!(env!("OUT_DIR"), "/bget.service.rs"));
}

pub trait P2PService {
    fn service_name(&self) -> String;
    fn serialize(&self) -> Vec<u8>;
    fn deserialize(&mut self, data: Vec<u8>) -> Result<(), BError>;
}

#[derive(Debug)]
pub struct SSProxyService {
    pub info: service::ProxyServiceInfo,
}

impl P2PService for SSProxyService {
    fn service_name(&self) -> String {
        String::from("ssproxy_service")
    }

    fn serialize(&self) -> Vec<u8> {
        self.info.encode_to_vec()
    }

    fn deserialize(&mut self, data: Vec<u8>) -> Result<(), BError> {
        let v: &[u8] = &data;
        match service::ProxyServiceInfo::decode(v) {
            Ok(info) => { 
                self.info = info;
                Ok(())
            },
            Err(e) => Err(BError::from(e)),
        }
    }
}

impl Default for SSProxyService {
    fn default() -> Self {
        SSProxyService {
            info: service::ProxyServiceInfo::default(),
        }
    }
}