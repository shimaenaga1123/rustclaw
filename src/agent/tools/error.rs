use thiserror::Error;

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("Command execution failed: {0}")]
    CommandFailed(String),
    #[error("Memory operation failed: {0}")]
    MemoryFailed(String),
    #[error("Search failed: {0}")]
    SearchFailed(String),
    #[error("Schedule operation failed: {0}")]
    ScheduleFailed(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Timeout")]
    Timeout,
}
