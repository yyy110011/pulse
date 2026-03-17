use std::time::Duration;

use tokio::time::timeout;

use crate::metrics::{exec_remote_cmd, SharedHandle, SharedSession};

/// A single disk mount point entry.
pub struct DiskEntry {
    pub mount: String,   // "/"
    pub size: String,    // "460G"
    pub used: String,    // "12G"
    pub avail: String,   // "117G"
    pub percent: u8,     // 10
}

const COLLECT_INTERVAL: Duration = Duration::from_secs(10);
const CMD_TIMEOUT: Duration = Duration::from_secs(5);

/// Spawn a background task that periodically collects disk usage via SSH.
pub fn spawn_disk_collector(
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

            // Mark loading
            {
                let mut data = session_data.lock().await;
                data.disk_loading = true;
            }

            let cmd = "df -h --output=target,size,used,avail,pcent -x tmpfs -x devtmpfs";
            match timeout(CMD_TIMEOUT, exec_remote_cmd(&handle, cmd)).await {
                Ok(Some(output)) => {
                    let entries = parse_df_output(&output);
                    let mut data = session_data.lock().await;
                    data.disks = Some(entries);
                    data.disk_loading = false;
                }
                Ok(None) => {
                    // Command failed — keep previous data
                    let mut data = session_data.lock().await;
                    data.disk_loading = false;
                }
                Err(_) => {
                    // Timeout — possibly NFS hung mount, keep previous data
                    eprintln!("[disk_info] timeout collecting disk usage");
                    let mut data = session_data.lock().await;
                    data.disk_loading = false;
                }
            }

            tokio::time::sleep(COLLECT_INTERVAL).await;
        }
    });
}

/// Parse the output of `df -h --output=target,size,used,avail,pcent`.
fn parse_df_output(output: &str) -> Vec<DiskEntry> {
    let mut entries = Vec::new();

    for line in output.lines().skip(1) {
        // skip header
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 5 {
            continue;
        }

        // The last field is the percentage, e.g. "10%"
        let percent_str = parts[parts.len() - 1].trim_end_matches('%');
        let percent: u8 = percent_str.parse().unwrap_or(0);

        // The mount point could contain spaces (unlikely but possible).
        // With --output format: target is first, then size, used, avail, pcent.
        // However if the mount path has spaces, we need to handle it.
        // In practice: target size used avail pcent
        // We take the last 4 fields as size/used/avail/pcent, rest as mount.
        let n = parts.len();
        let mount = if n > 5 {
            // Mount point contains spaces
            parts[..n - 4].join(" ")
        } else {
            parts[0].to_string()
        };

        entries.push(DiskEntry {
            mount,
            size: parts[n - 4].to_string(),
            used: parts[n - 3].to_string(),
            avail: parts[n - 2].to_string(),
            percent,
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
    fn test_parse_df_output_normal() {
        let output = "\
Mounted on      Size  Used Avail Use%
/               460G   12G  117G  10%
/boot           1.0G  200M  800M  20%
/home           200G   50G  150G  25%
";
        let entries = parse_df_output(output);
        assert_eq!(entries.len(), 3);

        assert_eq!(entries[0].mount, "/");
        assert_eq!(entries[0].size, "460G");
        assert_eq!(entries[0].used, "12G");
        assert_eq!(entries[0].avail, "117G");
        assert_eq!(entries[0].percent, 10);

        assert_eq!(entries[1].mount, "/boot");
        assert_eq!(entries[1].percent, 20);

        assert_eq!(entries[2].mount, "/home");
        assert_eq!(entries[2].size, "200G");
    }

    #[test]
    fn test_parse_df_output_empty() {
        let output = "Mounted on      Size  Used Avail Use%\n";
        let entries = parse_df_output(output);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_df_output_single_line() {
        let output = "\
Mounted on  Size  Used Avail Use%
/           50G   10G  40G   20%
";
        let entries = parse_df_output(output);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].mount, "/");
        assert_eq!(entries[0].percent, 20);
    }

    #[test]
    fn test_parse_df_output_invalid_percent() {
        let output = "\
Mounted on  Size  Used Avail Use%
/           50G   10G  40G   -
";
        let entries = parse_df_output(output);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].percent, 0);
    }
}
