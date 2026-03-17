use std::time::Duration;

use tokio::time::timeout;

use crate::metrics::{exec_remote_cmd, SharedHandle, SharedSession};

/// Static system information collected once on connect.
pub struct SystemInfo {
    pub os: String,       // "Ubuntu 22.04 LTS"
    pub kernel: String,   // "5.15.0-91-generic"
    pub hostname: String,
    pub uptime: String,   // "45d 3h"
    pub cpu_info: String, // "8 cores x86_64"
    pub ram_total: String, // "16 GB"
}

const CMD_TIMEOUT: Duration = Duration::from_secs(5);

/// Spawn a one-shot background task that collects system info.
pub fn spawn_system_info_collector(
    handle: SharedHandle,
    session_data: SharedSession,
    rt: tokio::runtime::Handle,
) {
    rt.spawn(async move {
        let info = collect_system_info(&handle).await;
        let mut data = session_data.lock().await;
        data.system_info = Some(info);
    });
}

async fn collect_system_info(handle: &SharedHandle) -> SystemInfo {
    let os = match timeout(CMD_TIMEOUT, exec_remote_cmd(handle, "cat /etc/os-release")).await {
        Ok(Some(output)) => parse_os_release(&output),
        _ => "Unknown".to_string(),
    };

    let kernel = match timeout(CMD_TIMEOUT, exec_remote_cmd(handle, "uname -r")).await {
        Ok(Some(output)) => output.trim().to_string(),
        _ => "Unknown".to_string(),
    };

    let hostname = match timeout(CMD_TIMEOUT, exec_remote_cmd(handle, "hostname")).await {
        Ok(Some(output)) => output.trim().to_string(),
        _ => "Unknown".to_string(),
    };

    let uptime = match timeout(CMD_TIMEOUT, exec_remote_cmd(handle, "cat /proc/uptime")).await {
        Ok(Some(output)) => parse_uptime(&output),
        _ => "Unknown".to_string(),
    };

    // Collect nproc and arch in one go
    let nproc = match timeout(CMD_TIMEOUT, exec_remote_cmd(handle, "nproc")).await {
        Ok(Some(output)) => output.trim().to_string(),
        _ => "?".to_string(),
    };
    let arch = match timeout(CMD_TIMEOUT, exec_remote_cmd(handle, "uname -m")).await {
        Ok(Some(output)) => output.trim().to_string(),
        _ => "Unknown".to_string(),
    };
    let cpu_info = format!("{nproc} cores {arch}");

    let ram_total = match timeout(CMD_TIMEOUT, exec_remote_cmd(handle, "cat /proc/meminfo")).await {
        Ok(Some(output)) => parse_mem_total(&output),
        _ => "Unknown".to_string(),
    };

    SystemInfo {
        os,
        kernel,
        hostname,
        uptime,
        cpu_info,
        ram_total,
    }
}

/// Parse PRETTY_NAME from /etc/os-release.
fn parse_os_release(content: &str) -> String {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("PRETTY_NAME=") {
            return rest.trim_matches('"').to_string();
        }
    }
    "Unknown".to_string()
}

/// Parse /proc/uptime first field (seconds) into "Xd Xh" format.
fn parse_uptime(content: &str) -> String {
    let first_field = content.split_whitespace().next().unwrap_or("0");
    let secs: f64 = first_field.parse().unwrap_or(0.0);
    let total_secs = secs as u64;
    let days = total_secs / 86400;
    let hours = (total_secs % 86400) / 3600;
    if days > 0 {
        format!("{days}d {hours}h")
    } else {
        format!("{hours}h")
    }
}

/// Parse MemTotal from /proc/meminfo and format as "X GB".
fn parse_mem_total(content: &str) -> String {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            if let Some(kb_str) = rest.trim().split_whitespace().next() {
                if let Ok(kb) = kb_str.parse::<u64>() {
                    let gb = kb as f64 / 1_048_576.0;
                    if gb >= 1.0 {
                        return format!("{:.0} GB", gb);
                    } else {
                        let mb = kb / 1024;
                        return format!("{mb} MB");
                    }
                }
            }
        }
    }
    "Unknown".to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_os_release() {
        let content = r#"NAME="Ubuntu"
VERSION="22.04.3 LTS (Jammy Jellyfish)"
PRETTY_NAME="Ubuntu 22.04.3 LTS"
VERSION_ID="22.04"
"#;
        assert_eq!(parse_os_release(content), "Ubuntu 22.04.3 LTS");
    }

    #[test]
    fn test_parse_os_release_missing() {
        assert_eq!(parse_os_release("NAME=\"Foo\"\n"), "Unknown");
    }

    #[test]
    fn test_parse_uptime_days_and_hours() {
        // 45 days 3 hours = 45*86400 + 3*3600 = 3898800
        assert_eq!(parse_uptime("3898800.45 12345.67"), "45d 3h");
    }

    #[test]
    fn test_parse_uptime_hours_only() {
        // 5 hours = 18000 seconds
        assert_eq!(parse_uptime("18000.00 1234.56"), "5h");
    }

    #[test]
    fn test_parse_uptime_zero() {
        assert_eq!(parse_uptime("0.00 0.00"), "0h");
    }

    #[test]
    fn test_parse_mem_total_gb() {
        let meminfo = "MemTotal:       16384000 kB\nMemFree:         2000000 kB\n";
        // 16384000 / 1048576 ≈ 15.625 → "16 GB"
        assert_eq!(parse_mem_total(meminfo), "16 GB");
    }

    #[test]
    fn test_parse_mem_total_small() {
        let meminfo = "MemTotal:       524288 kB\n";
        // 524288 / 1024 = 512 MB
        assert_eq!(parse_mem_total(meminfo), "512 MB");
    }

    #[test]
    fn test_parse_mem_total_missing() {
        assert_eq!(parse_mem_total("MemFree: 1000 kB\n"), "Unknown");
    }
}
