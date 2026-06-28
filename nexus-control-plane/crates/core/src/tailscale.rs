use serde::Deserialize;
use std::net::SocketAddr;
use std::process::Command;

/// Tailscale CLI service — resolves binary path and queries Tailscale status.
/// Follows the same pattern as GithubService.
pub struct TailscaleService {
    ts_path: Option<String>,
}

#[derive(Deserialize)]
struct TailscaleStatus {
    #[serde(rename = "BackendState")]
    backend_state: Option<String>,
    #[serde(rename = "Self")]
    self_node: Option<TailscaleSelf>,
}

#[derive(Deserialize)]
struct TailscaleSelf {
    #[serde(rename = "TailscaleIPs")]
    tailscale_ips: Option<Vec<String>>,
}

impl TailscaleService {
    /// Resolve Tailscale binary path at startup.
    ///
    /// 1. Platform-specific install paths
    /// 2. Bare `tailscale` (in PATH)
    /// 3. `$SHELL -l -c 'which tailscale'` (Unix only)
    /// 4. None
    pub fn new() -> Self {
        // 1. Platform-specific install paths
        #[cfg(target_os = "macos")]
        {
            let app_store_path = "/Applications/Tailscale.app/Contents/MacOS/Tailscale";
            if std::path::Path::new(app_store_path).exists() {
                return Self {
                    ts_path: Some(app_store_path.to_string()),
                };
            }
        }

        #[cfg(target_os = "windows")]
        {
            let program_files = std::env::var("ProgramFiles")
                .unwrap_or_else(|_| r"C:\Program Files".to_string());
            let win_path = format!(r"{}\Tailscale\tailscale.exe", program_files);
            if std::path::Path::new(&win_path).exists() {
                return Self {
                    ts_path: Some(win_path),
                };
            }
        }

        // 2. Bare tailscale (all platforms — works if in PATH)
        if Command::new("tailscale")
            .arg("version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Self {
                ts_path: Some("tailscale".to_string()),
            };
        }

        // 3. $SHELL fallback (Unix only)
        #[cfg(not(target_os = "windows"))]
        if let Ok(shell) = std::env::var("SHELL") {
            if let Ok(output) = Command::new(&shell)
                .args(["-l", "-c", "which tailscale"])
                .output()
            {
                if output.status.success() {
                    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !path.is_empty() {
                        return Self {
                            ts_path: Some(path),
                        };
                    }
                }
            }
        }

        Self { ts_path: None }
    }

    /// Get the Tailscale IPv4 address (100.x.x.x) if connected.
    /// Returns None with a reason string if unavailable.
    pub fn get_ip(&self) -> Result<String, String> {
        let ts_path = self
            .ts_path
            .as_ref()
            .ok_or_else(|| "Tailscale not found".to_string())?;

        let output = Command::new(ts_path)
            .args(["status", "--json"])
            .output()
            .map_err(|e| format!("Failed to run tailscale: {}", e))?;

        if !output.status.success() {
            return Err("Tailscale status command failed".to_string());
        }

        let status: TailscaleStatus = serde_json::from_slice(&output.stdout)
            .map_err(|e| format!("Failed to parse tailscale status: {}", e))?;

        // Check backend state
        match status.backend_state.as_deref() {
            Some("Running") => {}
            Some("NeedsLogin") => return Err("Tailscale needs login".to_string()),
            Some("Stopped") => return Err("Tailscale is stopped".to_string()),
            Some(state) => return Err(format!("Tailscale state: {}", state)),
            None => return Err("Tailscale state unknown".to_string()),
        }

        // Find first IPv4 (100.x.x.x)
        let ips = status
            .self_node
            .and_then(|s| s.tailscale_ips)
            .ok_or_else(|| "Tailscale not connected (no IPs)".to_string())?;

        ips.into_iter()
            .find(|ip| ip.starts_with("100."))
            .ok_or_else(|| "No Tailscale IPv4 address found".to_string())
    }
}

/// Resolve the bind address for the NexusLink server.
///
/// 1. NEXUSLINK_DEV env → 0.0.0.0:port
/// 2. Tailscale IP found → tailscale_ip:port
/// 3. Fallback → 127.0.0.1:port
pub fn resolve_bind_address(ts: &TailscaleService, port: u16) -> SocketAddr {
    // Dev mode: bind to all interfaces
    if std::env::var("NEXUSLINK_DEV").is_ok() {
        return SocketAddr::from(([0, 0, 0, 0], port));
    }

    // Try Tailscale IP
    if let Ok(ip) = ts.get_ip() {
        if let Ok(addr) = ip.parse::<std::net::Ipv4Addr>() {
            return SocketAddr::from((addr, port));
        }
    }

    // Fallback: localhost only
    SocketAddr::from(([127, 0, 0, 1], port))
}
