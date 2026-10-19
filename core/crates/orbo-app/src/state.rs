use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, RwLock};

use orbo_core::jobs::{JobId, JobQueue};

use crate::settings::Settings;

#[derive(Debug, Clone)]
pub struct DialogRequest {
    pub paths: Vec<PathBuf>,
    pub target: String,
    pub preset: Option<String>,
}

pub struct AppState {
    pub settings: RwLock<Settings>,
    pub queue: JobQueue,
    /// Title of the target and input size per job, for the toast.
    pub jobs: Mutex<HashMap<JobId, (String, u64)>>,
    pub dialog: Mutex<Option<DialogRequest>>,
}
