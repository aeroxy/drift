use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub target: Option<String>,
    pub password: Option<String>,
    pub root_dir: PathBuf,
    pub hostname: String,
}
