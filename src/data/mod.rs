pub mod loader;
pub mod models;
pub mod state;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON Parse Error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Data Logic Error: {0}")]
    Logic(String),
}
