use std::time::Duration;

use tokio::time::timeout;

use crate::metrics::{exec_remote_cmd, SharedHandle, SharedSession};

/// A single process entry from `ps aux`.
pub struct ProcessEntry {
    pub pid: u32,
    pub user: String,
    pub cpu: f32,
    pub mem: f32,
    pub command: String,
}

const COLLECT_INTERVAL: Duration = Duration::from_secs(2);
const CMD_TIMEOUT: Duration = Duration::from_secs(3);

/// Spawn a background task that periodically collects the top processes via SSH.
pub fn spawn_process_collector(
    handle: SharedHandle,
    session_data: SharedSession,
    rt: tokio::runtime::Handle,
) {
    rt.spawn(async move {
        loop {
            {
                let data = session_data.lock().await;
                if !data.state.is_connected() {
                    break;
                }
            }

            let cmd = "ps aux --sort=-%cpu | head -20";
            match timeout(CMD_TIMEOUT, exec_remote_cmd(&handle, cmd)).await {
                Ok(Some(output)) => {
                    let entries = parse_ps_output(&output);
                    let mut data = session_data.lock().await;
                    data.processes = Some(entries);
                }
                Ok(None) | Err(_) => {
                    // Command failed or timeout — keep previous data
                }
            }

            tokio::time::sleep(COLLECT_INTERVAL).await;
        }
    });
}

/// Parse the output of `ps aux --sort=-%cpu | head -20`.
/// Format: USER PID %CPU %MEM VSZ RSS TTY STAT START TIME COMMAND
fn parse_ps_output(output: &str) -> Vec<ProcessEntry> {
    let mut entries = Vec::new();

    for line in output.lines().skip(1) {
        // skip header
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // ps aux uses variable whitespace — collect the first 10 fields,
        // then everything remaining is COMMAND (which may contain spaces).
        let fields: Vec<&str> = line.split_whitespace().collect();
        // Need at least 11 fields: USER PID %CPU %MEM VSZ RSS TTY STAT START TIME COMMAND...
        if fields.len() < 11 {
            continue;
        }

        let user = fields[0].to_string();
        let pid: u32 = match fields[1].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let cpu: f32 = fields[2].parse().unwrap_or(0.0);
        let mem: f32 = fields[3].parse().unwrap_or(0.0);
        // COMMAND is fields[10..], joined by spaces
        let command = fields[10..].join(" ");

        entries.push(ProcessEntry {
            pid,
            user,
            cpu,
            mem,
            command,
        });
    }

    entries
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ps_output_normal() {
        let output = "\
USER         PID %CPU %MEM    VSZ   RSS TTY      STAT START   TIME COMMAND
root           1  0.0  0.1 169344 13200 ?        Ss   Feb20   0:15 /sbin/init
www-data    1234 25.3  5.2 450000 53000 ?        Sl   10:00   1:23 /usr/sbin/apache2 -k start
testuser    5678  1.2  0.3  22000  3000 pts/0    S+   10:05   0:00 vim test.txt
";
        let entries = parse_ps_output(output);
        assert_eq!(entries.len(), 3);

        assert_eq!(entries[0].pid, 1);
        assert_eq!(entries[0].user, "root");
        assert!((entries[0].cpu - 0.0).abs() < 0.01);
        assert_eq!(entries[0].command, "/sbin/init");

        assert_eq!(entries[1].pid, 1234);
        assert_eq!(entries[1].user, "www-data");
        assert!((entries[1].cpu - 25.3).abs() < 0.01);
        assert!((entries[1].mem - 5.2).abs() < 0.01);
        assert_eq!(entries[1].command, "/usr/sbin/apache2 -k start");
    }

    #[test]
    fn test_parse_ps_output_empty() {
        let output = "USER         PID %CPU %MEM    VSZ   RSS TTY      STAT START   TIME COMMAND\n";
        let entries = parse_ps_output(output);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_ps_output_header_only() {
        let output = "USER         PID %CPU %MEM    VSZ   RSS TTY      STAT START   TIME COMMAND";
        let entries = parse_ps_output(output);
        assert!(entries.is_empty());
    }
}
