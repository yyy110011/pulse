use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::session::SessionData;

const MAX_SAMPLES: usize = 60;

/// Which metric to display in tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricType {
    Cpu,
    Memory,
    Network,
}

impl MetricType {
    pub fn next(self) -> Self {
        match self {
            MetricType::Cpu => MetricType::Memory,
            MetricType::Memory => MetricType::Network,
            MetricType::Network => MetricType::Cpu,
        }
    }
}

impl fmt::Display for MetricType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetricType::Cpu => write!(f, "CPU"),
            MetricType::Memory => write!(f, "MEM"),
            MetricType::Network => write!(f, "NET"),
        }
    }
}

/// Holds time-series metric data for one session.
pub struct MetricsData {
    pub cpu: VecDeque<f64>,
    pub memory: VecDeque<f64>,
    pub network: VecDeque<f64>,
    /// Previous CPU idle and total for delta computation.
    prev_cpu: Option<(u64, u64)>,
    /// Previous network bytes for delta computation.
    prev_net_bytes: Option<u64>,
}

impl MetricsData {
    pub fn new() -> Self {
        Self {
            cpu: VecDeque::with_capacity(MAX_SAMPLES),
            memory: VecDeque::with_capacity(MAX_SAMPLES),
            network: VecDeque::with_capacity(MAX_SAMPLES),
            prev_cpu: None,
            prev_net_bytes: None,
        }
    }

    /// Get the data series for a given metric type.
    pub fn series(&self, metric: MetricType) -> &VecDeque<f64> {
        match metric {
            MetricType::Cpu => &self.cpu,
            MetricType::Memory => &self.memory,
            MetricType::Network => &self.network,
        }
    }

    /// Get the latest value for a metric, or None if no data.
    pub fn latest(&self, metric: MetricType) -> Option<f64> {
        self.series(metric).back().copied()
    }

    fn push_sample(deque: &mut VecDeque<f64>, value: f64) {
        if deque.len() >= MAX_SAMPLES {
            deque.pop_front();
        }
        deque.push_back(value);
    }

    /// Parse `/proc/stat` output and push a CPU usage sample.
    pub fn update_cpu(&mut self, proc_stat: &str) {
        if let Some(usage) = parse_cpu_stat(proc_stat, &mut self.prev_cpu) {
            Self::push_sample(&mut self.cpu, usage);
        }
    }

    /// Parse `/proc/meminfo` output and push a memory usage sample.
    pub fn update_memory(&mut self, proc_meminfo: &str) {
        if let Some(usage) = parse_meminfo(proc_meminfo) {
            Self::push_sample(&mut self.memory, usage);
        }
    }

    /// Parse `/proc/net/dev` output and push a network throughput sample.
    pub fn update_network(&mut self, proc_net_dev: &str) {
        if let Some(throughput) = parse_net_dev(proc_net_dev, &mut self.prev_net_bytes) {
            Self::push_sample(&mut self.network, throughput);
        }
    }
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

/// Parse the first `cpu ` line from `/proc/stat`.
/// Returns CPU usage percentage (0-100) as a delta from previous sample.
fn parse_cpu_stat(content: &str, prev: &mut Option<(u64, u64)>) -> Option<f64> {
    // Format: cpu  user nice system idle iowait irq softirq steal guest guest_nice
    let line = content.lines().find(|l| l.starts_with("cpu "))?;
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1) // skip "cpu"
        .filter_map(|s| s.parse().ok())
        .collect();

    if fields.len() < 4 {
        return None;
    }

    let idle = fields[3];
    let total: u64 = fields.iter().sum();

    let result = if let Some((prev_idle, prev_total)) = *prev {
        let d_total = total.saturating_sub(prev_total);
        let d_idle = idle.saturating_sub(prev_idle);
        if d_total == 0 {
            0.0
        } else {
            ((d_total - d_idle) as f64 / d_total as f64) * 100.0
        }
    } else {
        // First sample — no delta possible, skip
        *prev = Some((idle, total));
        return None;
    };

    *prev = Some((idle, total));
    Some(result)
}

/// Parse `/proc/meminfo` to compute memory usage percentage.
fn parse_meminfo(content: &str) -> Option<f64> {
    let mut mem_total: Option<u64> = None;
    let mut mem_available: Option<u64> = None;

    for line in content.lines() {
        if line.starts_with("MemTotal:") {
            mem_total = line.split_whitespace().nth(1).and_then(|v| v.parse().ok());
        } else if line.starts_with("MemAvailable:") {
            mem_available = line.split_whitespace().nth(1).and_then(|v| v.parse().ok());
        }
        if mem_total.is_some() && mem_available.is_some() {
            break;
        }
    }

    let total = mem_total?;
    let available = mem_available?;
    if total == 0 {
        return None;
    }
    Some(((total - available) as f64 / total as f64) * 100.0)
}

/// Parse `/proc/net/dev` to compute throughput (KB/s as delta from previous sample).
fn parse_net_dev(content: &str, prev_bytes: &mut Option<u64>) -> Option<f64> {
    let mut total_bytes: u64 = 0;

    for line in content.lines() {
        let line = line.trim();
        // Skip header lines and loopback
        if !line.contains(':') || line.starts_with("Inter") || line.starts_with("face") {
            continue;
        }

        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 2 {
            continue;
        }

        let iface = parts[0].trim();
        if iface == "lo" {
            continue;
        }

        let fields: Vec<u64> = parts[1]
            .split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect();

        if fields.len() >= 9 {
            // fields[0] = rx_bytes, fields[8] = tx_bytes
            total_bytes += fields[0] + fields[8];
        }
    }

    let result = if let Some(prev) = *prev_bytes {
        let delta = total_bytes.saturating_sub(prev);
        // Convert to KB/s (assuming ~2s interval, but we report raw delta / 2)
        (delta as f64) / 2.0 / 1024.0
    } else {
        *prev_bytes = Some(total_bytes);
        return None;
    };

    *prev_bytes = Some(total_bytes);
    Some(result)
}

// ---------------------------------------------------------------------------
// Metrics collector task
// ---------------------------------------------------------------------------

use russh::client;
use crate::session::SshClientHandler;

pub type SharedHandle = Arc<Mutex<client::Handle<SshClientHandler>>>;
pub type SharedSession = Arc<Mutex<SessionData>>;

/// Spawn a background task that periodically collects metrics via SSH.
pub fn spawn_metrics_collector(
    handle: SharedHandle,
    session_data: SharedSession,
    rt: tokio::runtime::Handle,
) {
    rt.spawn(async move {
        // Give the shell a moment to settle
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        loop {
            // Collect CPU
            if let Some(output) = exec_remote_cmd(&handle, "cat /proc/stat").await {
                let mut data = session_data.lock().await;
                data.metrics.update_cpu(&output);
            }

            // Collect Memory
            if let Some(output) = exec_remote_cmd(&handle, "cat /proc/meminfo").await {
                let mut data = session_data.lock().await;
                data.metrics.update_memory(&output);
            }

            // Collect Network
            if let Some(output) = exec_remote_cmd(&handle, "cat /proc/net/dev").await {
                let mut data = session_data.lock().await;
                data.metrics.update_network(&output);
            }

            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            // Check if session is still connected
            {
                let data = session_data.lock().await;
                if !data.state.is_connected() {
                    break;
                }
            }
        }
    });
}

/// Execute a command on the remote host via a new channel and return stdout.
pub async fn exec_remote_cmd(handle: &SharedHandle, cmd: &str) -> Option<String> {
    let mut channel = {
        let h = handle.lock().await;
        h.channel_open_session().await.ok()?
    };

    channel.exec(true, cmd).await.ok()?;

    let mut output = Vec::new();
    loop {
        match channel.wait().await {
            Some(russh::ChannelMsg::Data { data }) => {
                output.extend_from_slice(&data);
            }
            Some(russh::ChannelMsg::Eof) | Some(russh::ChannelMsg::ExitStatus { .. }) => {}
            None => break,
            _ => {}
        }
    }

    Some(String::from_utf8_lossy(&output).to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cpu_stat_first_sample_returns_none() {
        let stat = "cpu  10132153 290696 3084719 46828483 16683 0 25195 0 0 0\n";
        let mut prev = None;
        assert!(parse_cpu_stat(stat, &mut prev).is_none());
        assert!(prev.is_some());
    }

    #[test]
    fn test_parse_cpu_stat_second_sample() {
        let stat1 = "cpu  10132153 290696 3084719 46828483 16683 0 25195 0 0 0\n";
        let stat2 = "cpu  10132253 290696 3084719 46828583 16683 0 25195 0 0 0\n";
        let mut prev = None;
        parse_cpu_stat(stat1, &mut prev);
        let usage = parse_cpu_stat(stat2, &mut prev).unwrap();
        // 100 user increase, 100 idle increase => 200 total delta, 100 non-idle => 50%
        assert!((usage - 50.0).abs() < 0.1);
    }

    #[test]
    fn test_parse_meminfo() {
        let meminfo = "\
MemTotal:       16384000 kB
MemFree:         2000000 kB
MemAvailable:    8192000 kB
Buffers:          500000 kB
";
        let usage = parse_meminfo(meminfo).unwrap();
        // (16384000 - 8192000) / 16384000 * 100 = 50%
        assert!((usage - 50.0).abs() < 0.1);
    }

    #[test]
    fn test_parse_net_dev() {
        let dev1 = "\
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 1000     10    0    0    0     0          0         0     1000     10    0    0    0     0       0          0
  eth0: 50000    100    0    0    0     0          0         0    30000     80    0    0    0     0       0          0
";
        let dev2 = "\
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 2000     20    0    0    0     0          0         0     2000     20    0    0    0     0       0          0
  eth0: 60000    110    0    0    0     0          0         0    40000     90    0    0    0     0       0          0
";
        let mut prev = None;
        assert!(parse_net_dev(dev1, &mut prev).is_none()); // first sample
        let throughput = parse_net_dev(dev2, &mut prev).unwrap();
        // eth0: delta = (60000+40000) - (50000+30000) = 20000 bytes
        // KB/s = 20000 / 2 / 1024 ≈ 9.77
        assert!((throughput - 9.77).abs() < 0.1);
    }

    #[test]
    fn test_parse_meminfo_missing_fields() {
        let meminfo = "MemTotal:       16384000 kB\n";
        assert!(parse_meminfo(meminfo).is_none());
    }
}
