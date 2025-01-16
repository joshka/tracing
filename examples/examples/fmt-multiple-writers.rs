//! An example demonstrating how a `fmt` subscriber can write the same formatted
//! output to multiple destinations and conditionally disable one of them.
//!
//! By default, this writes only to the log file. Pass `--stdout` to also write
//! to stdout:
//!
//! ```text
//! cargo run --example fmt-multiple-writers -- --stdout
//! ```

#[path = "fmt/yak_shave.rs"]
mod yak_shave;

use std::io;
use tracing_subscriber::{fmt::writer::MakeWriterExt, EnvFilter};

fn main() {
    let dir = tempfile::tempdir().expect("Failed to create tempdir");

    let file_appender = tracing_appender::rolling::hourly(dir, "example.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    let enable_stdout = std::env::args_os().any(|arg| arg == "--stdout");
    let stdout = enable_stdout.then_some(io::stdout);
    let writer = non_blocking.and(stdout);

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::TRACE.into()))
        .with_writer(writer)
        .init();

    let number_of_yaks = 3;
    // this creates a new event, outside of any spans.
    tracing::info!(number_of_yaks, "preparing to shave yaks");

    let number_shaved = yak_shave::shave_all(number_of_yaks);
    tracing::info!(
        all_yaks_shaved = number_shaved == number_of_yaks,
        "yak shaving completed."
    );
}
