pub mod admin;
pub mod audit;
pub mod engine;
pub mod http;
pub mod media;
pub mod mqtt;
pub mod observability;
pub mod program;
pub mod resources;
pub mod syntax;
pub mod value;
pub mod websocket;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
}
impl Error {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Error {}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self::new("database", e.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::new("invalid_input", e.to_string())
    }
}

pub mod log_store;
pub(crate) mod telemetry;

pub mod projects;

pub(crate) mod dashboard;

pub mod server_state;
