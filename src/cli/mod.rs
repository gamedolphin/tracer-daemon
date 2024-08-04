// src/cli/mod.rs
use crate::{
    config_manager::ConfigManager,
    daemon_communication::client::{
        send_alert_request, send_end_run_request, send_log_request,
        send_log_short_lived_process_request, send_start_run_request, send_terminate_request,
        send_update_tags_request, send_upload_file_request,
    },
    process_watcher::ProcessWatcher,
    run, start_daemon,
    upload::upload_from_file_path,
    SOCKET_PATH,
};
use anyhow::{Ok, Result};

use clap::{Parser, Subcommand};
use nondaemon_commands::{
    clean_up_after_daemon, print_config_info_sync, setup_config, update_tracer,
};

use std::env;
use sysinfo::System;
mod nondaemon_commands;

#[derive(Parser)]
#[clap(
    name = "tracer",
    about = "A tool for monitoring bioinformatics applications",
    version = env!("CARGO_PKG_VERSION")
)]
pub struct Cli {
    #[clap(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Setup the configuration for the service, rewriting the config.toml file
    Setup {
        /// API key for the service
        #[clap(long, short)]
        api_key: Option<String>,
        /// URL of the service
        #[clap(long, short)]
        service_url: Option<String>,
        /// Interval in milliseconds for polling process information
        #[clap(long, short)]
        process_polling_interval_ms: Option<u64>,
        /// Interval in milliseconds for submitting batch data
        #[clap(long, short)]
        batch_submission_interval_ms: Option<u64>,
    },

    /// Log a message to the service
    Log { message: String },

    /// Send an alert to the service, sending an e-mail
    Alert { message: String },

    /// Start the daemon
    Init,

    /// Stop the daemon
    Terminate,

    /// Remove all the temporary files created by the daemon, in a case of the process being terminated unexpectedly
    Cleanup,

    /// Shows the current configuration and the daemon status
    Info,

    /// Update the daemon to the latest version
    Update,

    /// Start a new pipeline run
    Start,

    /// End the current pipeline run
    End,

    /// Test the configuration by sending a request to the service
    Test,

    /// Upload a file to the service [Works only directly from the function not the daemon]
    Upload { file_path: String },
    /// Upload a file to the service [Works only directly from the function not the daemon]
    UploadDaemon,

    /// Change the tags of the current pipeline run
    Tag { tags: Vec<String> },

    /// Configure .bashrc file to include aliases for short-lived processes commands. To use them, a new terminal session must be started.
    ApplyBashrc,

    /// Log a message to the service for a short-lived process.
    LogShortLivedProcess { command: String },

    /// Shows the current version of the daemon
    Version,
}

pub fn process_cli() -> Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Init => {
            let test_result = ConfigManager::test_service_config_sync();
            if test_result.is_err() {
                print_config_info_sync()?;
                return Ok(());
            }
            println!("Starting daemon...");
            let result = start_daemon();
            if result.is_err() {
                println!("Failed to start daemon. Maybe the daemon is already running? If it's not, run `tracer cleanup` to clean up the previous daemon files.");
                return Ok(());
            }
            run()?;
            clean_up_after_daemon()
        }
        Commands::Test => {
            let result = ConfigManager::test_service_config_sync();
            if result.is_ok() {
                println!("Tracer was able to successfully communicate with the API service.");
            }
            Ok(())
        }
        Commands::Cleanup => {
            let result = clean_up_after_daemon();
            if result.is_ok() {
                println!("Daemon files cleaned up successfully.");
            }
            result
        }
        Commands::ApplyBashrc => ConfigManager::setup_aliases(),
        Commands::Info => print_config_info_sync(),
        _ => run_async_command(cli.command),
    }
}

#[tokio::main]
pub async fn run_async_command(commands: Commands) -> Result<()> {
    let value = match commands {
        Commands::Log { message } => send_log_request(SOCKET_PATH, message).await,
        Commands::Alert { message } => send_alert_request(SOCKET_PATH, message).await,
        Commands::Terminate => send_terminate_request(SOCKET_PATH).await,
        Commands::Start => send_start_run_request(SOCKET_PATH).await,
        Commands::End => send_end_run_request(SOCKET_PATH).await,
        Commands::Update => update_tracer().await,
        Commands::Tag { tags } => send_update_tags_request(SOCKET_PATH, &tags).await,
        Commands::Setup {
            api_key,
            service_url,
            process_polling_interval_ms,
            batch_submission_interval_ms,
        } => {
            setup_config(
                &api_key,
                &service_url,
                &process_polling_interval_ms,
                &batch_submission_interval_ms,
            )
            .await
        }
        Commands::LogShortLivedProcess { command } => {
            let data = ProcessWatcher::gather_short_lived_process_data(&System::new(), &command);
            send_log_short_lived_process_request(SOCKET_PATH, data).await
        }

        Commands::Upload { file_path } => {
            upload_from_file_path(&file_path).await?;
            Ok(())
        }
        Commands::UploadDaemon => send_upload_file_request(SOCKET_PATH).await,
        _ => {
            println!("Command not implemented yet");
            Ok(())
        }
    };

    if value.is_err() {
        println!("Failed to send command to the daemon. Maybe the daemon is not running? If it's not, run `tracer init` to start the daemon.");
    } else {
        println!("Command sent successfully.")
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{remove_file, File};
    use std::io::{Seek, SeekFrom, Write};
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_upload_file_command() -> Result<()> {
        // Create a temporary directory
        let temp_dir = tempdir()?;
        let file_path = temp_dir.path().join("big_log_3.log");

        // Create a 4MB file
        let mut file = File::create(&file_path)?;
        let data = vec![b'A'; 1024 * 1024]; // 1MB of 'A' characters
        for _ in 0..4 {
            file.write_all(&data)?;
        }
        file.seek(SeekFrom::Start(0))?;

        // Attempt to upload the file
        let result = upload_from_file_path(file_path.to_str().unwrap()).await;

        // Clean up
        remove_file(&file_path)?;

        // Check the result
        assert!(result.is_ok(), "File upload failed: {:?}", result.err());

        Ok(())
    }
}
