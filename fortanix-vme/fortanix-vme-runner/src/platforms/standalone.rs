use crate::RunnerError;
use crate::platforms::{EnclaveRuntime, Platform};
use std::process::ExitStatus;
use std::os::unix::process::ExitStatusExt;

pub struct Standalone;

#[derive(Clone, Debug)]
pub struct StandaloneArgs;

pub struct StandaloneDescriptor;

impl Platform for Standalone {
    type RunArgs = StandaloneArgs;
    type EnclaveDescriptor = StandaloneDescriptor;

    fn run<I: Into<Self::RunArgs>>(_run_args: I) -> Result<Self::EnclaveDescriptor, RunnerError> {
        Ok(StandaloneDescriptor)
    }
}

impl EnclaveRuntime for StandaloneDescriptor {
    async fn wait(&mut self) -> Result<ExitStatus, RunnerError> {
        futures::future::pending::<()>().await;
        // This will never be reached.
        Ok(ExitStatus::from_raw(0))
    }
}
