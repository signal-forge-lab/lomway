//! Deployment-only alternate executable name; same CLI as lomway.exe.
//! Allows a verified release build without overwriting a running desktop server.
use clap::Parser;
use lomway::cli::{self, Cli};
use std::process::ExitCode;
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
