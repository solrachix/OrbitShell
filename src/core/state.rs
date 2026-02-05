use std::ops::Range;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandBlock {
    pub id: Uuid,
    pub input: String,
    pub output_range: Range<usize>,
    pub exit_code: Option<i32>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub struct SessionState {
    pub blocks: Vec<CommandBlock>,
    pub current_directory: std::path::PathBuf,
}

pub enum TerminalEvent {
    Output(Vec<u8>),
    CommandStarted(String),
    CommandFinished(Option<i32>),
    DirectoryChanged(std::path::PathBuf),
}
