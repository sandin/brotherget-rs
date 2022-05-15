
#[derive(Debug)]
pub enum BError {
    Io(std::io::Error),
    Reqwest(std::string::String),
    Tokio(std::string::String),
    Config(std::string::String),
    Proxy(std::string::String),
    Download(std::string::String),
    Proto(std::string::String),
    Other(std::string::String),
}

impl std::fmt::Display for BError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match *self {
            BError::Io(ref cause) => write!(f, "I/O Error: {}", cause),
            BError::Reqwest(ref cause) => write!(f, "Error: {}", cause),
            BError::Tokio(ref cause) => write!(f, "Error: {}", cause),
            BError::Config(ref cause) => write!(f, "Error: {}", cause),
            BError::Proxy(ref cause) => write!(f, "Error: {}", cause),
            BError::Download(ref cause) => write!(f, "Error: {}", cause),
            BError::Proto(ref cause) => write!(f, "Error: {}", cause),
            BError::Other(ref cause) => write!(f, "Unknown error: {}!", cause),
        }
    }
}

impl From<std::io::Error> for BError {
    fn from(error: std::io::Error) -> Self {
        BError::Io(error)
    }
}

impl From<reqwest::Error> for BError {
    fn from(error: reqwest::Error) -> Self {
        BError::Reqwest(error.to_string())
    }
}

impl From<reqwest::header::ToStrError> for BError {
    fn from(error: reqwest::header::ToStrError) -> Self {
        BError::Reqwest(error.to_string())
    }
}

impl From<tokio::task::JoinError> for BError {
    fn from(error: tokio::task::JoinError) -> Self {
        BError::Tokio(error.to_string())
    }
}

impl From<prost::DecodeError> for BError {
    fn from(error: prost::DecodeError) -> Self {
        BError::Proto(error.to_string())
    }
}