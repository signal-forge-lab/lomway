//! Binary entry point: parse the public CLI and dispatch with actionable
//! exit codes (`0` success, `1` failure; clap exits `2` on usage errors).
//!
//! Every core mode is implemented by the binary itself — no PowerShell or
//! any other shell is required on any platform.

use std::process::ExitCode;

use clap::Parser;
use lomway::cli::{self, Cli};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli::run(cli.command).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
