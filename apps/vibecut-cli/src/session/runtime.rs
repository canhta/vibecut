//! Configures the CLI engine runtime.
use crate::args::CliError;
use engine::ByteBudget;
use engine::jobs::{JobRunner, RunnerConfig, SystemClock};
use std::sync::Arc;
use std::time::Duration;

pub(crate) fn cli_runtime() -> Result<engine::providers::Runtime, CliError> {
    let mut runtime = engine::providers::Runtime::default();
    if let Ok(value) = std::env::var("VIBECUT_ASR_THREADS") {
        let threads = value.parse::<u8>().map_err(|_| {
            CliError::Usage("VIBECUT_ASR_THREADS must be an integer from 1 to 32".into())
        })?;
        if !(1..=32).contains(&threads) {
            return Err(CliError::Usage(
                "VIBECUT_ASR_THREADS must be an integer from 1 to 32".into(),
            ));
        }
        runtime.threads = threads;
    }
    if std::env::var("VIBECUT_ASR_GPU").is_ok_and(|value| value == "0") {
        runtime.use_gpu = false;
    }
    runtime.gipformer_runner = engine::providers::local_asr_runner();
    Ok(runtime)
}

pub(crate) fn cli_runner(budget: ByteBudget) -> Result<JobRunner, CliError> {
    Ok(JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 2],
            queue: 8,
            retained: 8,
            network_attempts: 3,
            backoff: Duration::from_millis(250),
            progress_interval: Duration::from_millis(50),
        },
        budget,
        Arc::new(SystemClock::default()),
    )?)
}
