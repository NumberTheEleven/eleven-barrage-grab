//! Edge process lifecycle management (auto-signer spec section 2.3)

use std::path::PathBuf;
use std::process::{Child, Command};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct BrowserConfig {
    pub edge_path: PathBuf,
    pub user_data_dir: PathBuf,
    pub extra_args: Vec<String>,
    pub cdp_port: u16,
}

impl BrowserConfig {
    pub fn build_args(&self) -> Vec<String> {
        let mut args = default_edge_args();
        args.push(format!("--remote-debugging-port={}", self.cdp_port));
        args.push(format!("--user-data-dir={}", self.user_data_dir.display()));
        for extra in &self.extra_args {
            args.push(extra.clone());
        }
        args.push("about:blank".into());
        args
    }
}

pub fn default_edge_args() -> Vec<String> {
    vec![
        "--headless=new".into(),
        "--disable-blink-features=AutomationControlled".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--disable-background-timer-throttling".into(),
        "--disable-backgrounding-occluded-windows".into(),
        "--disable-renderer-backgrounding".into(),
        "--window-size=1920,1080".into(),
    ]
}

pub struct Browser {
    pub process: Child,
    pub cdp_port: u16,
    pub cdp_ws_url: String,
    pub config: BrowserConfig,
}

impl Browser {
    pub fn spawn(config: BrowserConfig) -> Result<Self> {
        let args = config.build_args();
        let process = Command::new(&config.edge_path)
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .with_context(|| format!("failed to spawn edge at {:?}", config.edge_path))?;

        Ok(Self {
            process,
            cdp_port: config.cdp_port,
            cdp_ws_url: String::new(), // populated by discover_cdp_url
            config,
        })
    }

    pub fn kill(&mut self) -> Result<()> {
        self.process.kill().context("failed to kill browser process")?;
        self.process.wait().context("failed to wait browser process")?;
        Ok(())
    }

    pub fn is_alive(&mut self) -> bool {
        match self.process.try_wait() {
            Ok(Some(_)) => false,
            Ok(None) => true,
            Err(_) => false,
        }
    }

    /// Poll the CDP discovery endpoint until Edge reports a browser WebSocket URL.
    pub async fn discover_cdp_url(&mut self) -> Result<String> {
        let url = format!("http://127.0.0.1:{}/json/version", self.cdp_port);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()?;
        for _ in 0..30 {
            if let Ok(resp) = client.get(&url).send().await {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    if let Some(ws) = json.get("webSocketDebuggerUrl").and_then(|v| v.as_str()) {
                        self.cdp_ws_url = ws.to_string();
                        return Ok(ws.to_string());
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        anyhow::bail!("CDP discovery timed out on port {}", self.cdp_port);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn build_command_args_includes_headless_and_anti_detection() {
        let config = BrowserConfig {
            edge_path: PathBuf::from("msedge.exe"),
            user_data_dir: PathBuf::from("/tmp/profile"),
            extra_args: vec!["--my-arg".into()],
            cdp_port: 9222,
        };
        let args = config.build_args();
        assert!(args.contains(&"--headless=new".to_string()));
        assert!(args.contains(&"--disable-blink-features=AutomationControlled".to_string()));
        assert!(args.contains(&"--remote-debugging-port=9222".to_string()));
        assert!(args.contains(&"--user-data-dir=/tmp/profile".to_string()));
        assert!(args.contains(&"--my-arg".to_string()));
    }

    #[test]
    fn default_edge_args_are_correct() {
        let args = default_edge_args();
        assert!(args.iter().any(|a| a == "--headless=new"));
        assert!(args.iter().any(|a| a.starts_with("--disable-blink-features")));
        assert!(args.iter().any(|a| a == "--no-first-run"));
    }

    #[test]
    fn is_alive_returns_false_after_kill() {
        // Use a long-running sleep command as a stand-in for msedge.
        // This proves the is_alive() mechanism works without needing real Edge.
        let mut browser = Browser {
            process: std::process::Command::new(if cfg!(windows) { "cmd.exe" } else { "sh" })
                .arg(if cfg!(windows) { "/c" } else { "-c" })
                .arg(if cfg!(windows) { "ping -n 30 127.0.0.1 > NUL" } else { "sleep 30" })
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("failed to spawn test process"),
            cdp_port: 0,
            cdp_ws_url: String::new(),
            config: BrowserConfig {
                edge_path: PathBuf::from("dummy"),
                user_data_dir: PathBuf::from("dummy"),
                extra_args: vec![],
                cdp_port: 0,
            },
        };
        assert!(browser.is_alive());
        browser.kill().expect("kill failed");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(!browser.is_alive());
    }
}