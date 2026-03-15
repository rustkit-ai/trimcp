use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Config error: {0}")]
    Config(String),

    #[error("Proxy error: {0}")]
    Proxy(String),

    #[error("Upstream process error: {0}")]
    Upstream(String),
}

pub type Result<T> = std::result::Result<T, Error>;
