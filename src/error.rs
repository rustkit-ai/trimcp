#![allow(dead_code)]

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_error_displays_message() {
        let err = Error::Config("missing field: upstream".to_string());
        assert_eq!(err.to_string(), "Config error: missing field: upstream");
    }

    #[test]
    fn test_proxy_error_displays_message() {
        let err = Error::Proxy("upstream disconnected".to_string());
        assert_eq!(err.to_string(), "Proxy error: upstream disconnected");
    }

    #[test]
    fn test_upstream_error_displays_message() {
        let err = Error::Upstream("process exited with code 1".to_string());
        assert_eq!(
            err.to_string(),
            "Upstream process error: process exited with code 1"
        );
    }

    #[test]
    fn test_json_error_converts_from_serde() {
        let serde_err = serde_json::from_str::<serde_json::Value>("invalid json").unwrap_err();
        let err: Error = serde_err.into();
        assert!(err.to_string().starts_with("JSON error:"));
    }

    #[test]
    fn test_result_type_alias() {
        let ok: Result<i32> = Ok(42);
        assert_eq!(ok.unwrap(), 42);

        let err: Result<i32> = Err(Error::Proxy("test".to_string()));
        assert!(err.is_err());
    }
}
