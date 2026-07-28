use std::path::Path;

use log::LevelFilter;

pub fn init(log_file: Option<&Path>, level: Option<LevelFilter>) {
    let level = level.unwrap_or(LevelFilter::Warn);
    let mut builder = env_logger::Builder::new();
    builder.filter_level(level);
    if let Some(path) = log_file
        && let Ok(file) = std::fs::File::create(path)
    {
        builder.target(env_logger::Target::Pipe(Box::new(file)));
    }
    let _ = builder.try_init();
}
