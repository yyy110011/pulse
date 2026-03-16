use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct SshHost {
    pub name: String,
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
}

impl SshHost {
    /// Returns the effective hostname for connection (hostname or name as fallback).
    pub fn effective_hostname(&self) -> &str {
        self.hostname.as_deref().unwrap_or(&self.name)
    }

    /// Returns the effective port (default 22).
    pub fn effective_port(&self) -> u16 {
        self.port.unwrap_or(22)
    }

    /// Returns a display string like "user@hostname:port"
    pub fn display_connection(&self) -> String {
        let host = self.effective_hostname();
        let port = self.effective_port();
        match &self.user {
            Some(user) if port != 22 => format!("{user}@{host}:{port}"),
            Some(user) => format!("{user}@{host}"),
            None if port != 22 => format!("{host}:{port}"),
            None => host.to_string(),
        }
    }
}

/// Parse ~/.ssh/config and return a list of SshHost entries.
/// Skips wildcard hosts (e.g. `Host *`).
pub fn parse_ssh_config() -> Vec<SshHost> {
    let path = default_config_path();
    parse_ssh_config_from_path(&path)
}

fn default_config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".ssh").join("config")
}

pub fn parse_ssh_config_from_path(path: &PathBuf) -> Vec<SshHost> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    parse_ssh_config_content(&content)
}

fn parse_ssh_config_content(content: &str) -> Vec<SshHost> {
    let mut hosts: Vec<SshHost> = Vec::new();
    let mut current: Option<HashMap<String, String>> = None;
    let mut current_name: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Split on first whitespace or '='
        let (key, value) = match trimmed.split_once(|c: char| c.is_whitespace() || c == '=') {
            Some((k, v)) => (k.trim().to_lowercase(), v.trim().to_string()),
            None => continue,
        };

        if key == "host" {
            // Save previous host
            if let (Some(name), Some(props)) = (current_name.take(), current.take()) {
                if name != "*" && !name.contains('*') && !name.contains('?') {
                    hosts.push(build_host(name, &props));
                }
            }
            // Start new host block
            current_name = Some(value);
            current = Some(HashMap::new());
        } else if let Some(ref mut props) = current {
            props.insert(key, value);
        }
    }

    // Don't forget the last host
    if let (Some(name), Some(props)) = (current_name, current) {
        if name != "*" && !name.contains('*') && !name.contains('?') {
            hosts.push(build_host(name, &props));
        }
    }

    hosts
}

fn build_host(name: String, props: &HashMap<String, String>) -> SshHost {
    SshHost {
        name,
        hostname: props.get("hostname").cloned(),
        user: props.get("user").cloned(),
        port: props.get("port").and_then(|p| p.parse().ok()),
        identity_file: props.get("identityfile").cloned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic_config() {
        let content = r#"
Host myserver
  HostName 192.168.1.100
  User admin
  Port 2222
  IdentityFile ~/.ssh/id_rsa

Host dev-box
  HostName dev.example.com
  User developer

Host *
  ServerAliveInterval 60
"#;
        let hosts = parse_ssh_config_content(content);
        assert_eq!(hosts.len(), 2);

        assert_eq!(hosts[0].name, "myserver");
        assert_eq!(hosts[0].hostname.as_deref(), Some("192.168.1.100"));
        assert_eq!(hosts[0].user.as_deref(), Some("admin"));
        assert_eq!(hosts[0].port, Some(2222));

        assert_eq!(hosts[1].name, "dev-box");
        assert_eq!(hosts[1].hostname.as_deref(), Some("dev.example.com"));
        assert_eq!(hosts[1].user.as_deref(), Some("developer"));
        assert_eq!(hosts[1].port, None);
        assert_eq!(hosts[1].effective_port(), 22);
    }

    #[test]
    fn test_display_connection() {
        let host = SshHost {
            name: "test".to_string(),
            hostname: Some("10.0.0.1".to_string()),
            user: Some("admin".to_string()),
            port: Some(2222),
            identity_file: None,
        };
        assert_eq!(host.display_connection(), "admin@10.0.0.1:2222");

        let host2 = SshHost {
            name: "test2".to_string(),
            hostname: Some("example.com".to_string()),
            user: Some("root".to_string()),
            port: None,
            identity_file: None,
        };
        assert_eq!(host2.display_connection(), "root@example.com");
    }
}
