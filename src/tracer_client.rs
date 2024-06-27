/// src/tracer_client.rs
use anyhow::Result;
use serde_json::json;
use std::{time::Duration, time::Instant};
use sysinfo::System;

use crate::config_manager::ConfigFile;
use crate::event_recorder::EventRecorder;
use crate::http_client::HttpClient;
use crate::metrics::SystemMetricsCollector;
use crate::process_watcher::ProcessWatcher;

pub struct TracerClient {
    http_client: HttpClient,
    api_key: String,
    system: System,
    service_url: String,
    last_sent: Instant,
    interval: Duration,
    logs: EventRecorder,
    process_watcher: ProcessWatcher,
    metrics_collector: SystemMetricsCollector,
}

impl TracerClient {
    pub fn new(config: ConfigFile) -> Result<TracerClient> {
        let service_url = config.service_url.clone();

        println!("Initializing TracerClient with API Key: {}", config.api_key);
        println!("Service URL: {}", service_url);

        Ok(TracerClient {
            http_client: HttpClient::new(service_url.clone(), config.api_key.clone()),
            api_key: config.api_key,
            system: System::new_all(),
            last_sent: Instant::now(),
            interval: Duration::from_millis(config.process_polling_interval_ms),
            logs: EventRecorder::new(),
            service_url,
            process_watcher: ProcessWatcher::new(config.targets),
            metrics_collector: SystemMetricsCollector::new(),
        })
    }

    pub async fn submit_batched_data(&mut self) -> Result<()> {
        if Instant::now() - self.last_sent >= self.interval {
            self.metrics_collector
                .collect_metrics(&mut self.system, &mut self.logs)?;
            println!(
                "Sending event to {} with API Key: {}",
                self.service_url, self.api_key
            );

            let data = json!({ "logs": self.logs.get_events() });

            println!("{:#?}", data); // Log to file located at `/tmp/tracerd.out`

            self.last_sent = Instant::now();
            self.logs.clear();

            self.http_client.send_http_event(&data).await
        } else {
            Ok(())
        }
    }

    pub async fn poll_processes(&mut self) -> Result<()> {
        self.process_watcher
            .poll_processes(&mut self.system, &mut self.logs)?;
        Ok(())
    }

    pub async fn remove_completed_processes(&mut self) -> Result<()> {
        self.process_watcher
            .remove_completed_processes(&mut self.system, &mut self.logs)?;
        Ok(())
    }

    pub fn refresh(&mut self) {
        self.system.refresh_all();
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::config_manager::ConfigManager;
    use std::fs::{self, File};
    use std::io::Write;
    use tempfile::TempDir;

    const CONFIG_CONTENT: &str = r#"
        api_key = "test_api_key"
        process_polling_interval_ms = 200
        batch_submission_interval_ms = 5000
        service_url = "https://app.tracer.bio/api/data-collector-api"
        targets = ["target1", "target2"]
    "#;

    fn create_test_config(content: &str, path: &str) {
        let mut file = File::create(path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn test_new() {
        let temp_dir = TempDir::new().unwrap();
        let test_config_path = temp_dir.path().join("test_tracer.toml");
        create_test_config(CONFIG_CONTENT, test_config_path.to_str().unwrap());

        std::env::set_var("TRACER_CONFIG", test_config_path.to_str().unwrap());
        let config = ConfigManager::load_config().expect("Failed to load config");
        std::env::remove_var("TRACER_CONFIG");

        let tr = TracerClient::new(config);
        assert!(tr.is_ok())
    }

    #[tokio::test]
    async fn test_tool_exec() {
        let temp_dir = TempDir::new().unwrap();
        let test_config_path = temp_dir.path().join("test_tracer.toml");
        create_test_config(CONFIG_CONTENT, test_config_path.to_str().unwrap());

        std::env::set_var("TRACER_CONFIG", test_config_path.to_str().unwrap());
        let config = ConfigManager::load_config().expect("Failed to load config");
        std::env::remove_var("TRACER_CONFIG");

        let mut tr = TracerClient::new(config).unwrap();
        tr.process_watcher = ProcessWatcher::new(vec!["sleep".to_string()]);

        let mut cmd = std::process::Command::new("sleep")
            .arg("1")
            .spawn()
            .unwrap();

        while tr.process_watcher.get_seen().is_empty() {
            tr.refresh();
            tr.poll_processes().await.unwrap();
        }

        cmd.wait().unwrap();

        assert!(!tr.process_watcher.get_seen().is_empty())
    }
}
