//! Resolution of runtime settings from four sources.
//!
//! Precedence (highest first): command line, environment, config file, built-in
//! defaults.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::cli::{DEFAULT_FAIL_THRESHOLD, DEFAULT_INTERVAL, DEFAULT_TIMEOUT};

/// Environment variable prefix for all overrides.
pub const ENV_PREFIX: &str = "TS_KEEPALIVE_";

/// Values that may come from the config file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    pub target: Option<String>,
    pub interval: Option<u64>,
    pub timeout: Option<u64>,
    pub fail_threshold: Option<u32>,
    pub log_file: Option<PathBuf>,
    pub verbose: Option<bool>,
}

/// One layer of overrides. Unset fields fall through to lower layers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layer {
    pub target: Option<String>,
    pub interval: Option<u64>,
    pub timeout: Option<u64>,
    pub fail_threshold: Option<u32>,
    pub log_file: Option<PathBuf>,
    pub verbose: Option<bool>,
}

impl Layer {
    /// Apply `higher` on top of `self`; fields present in `higher` win.
    pub fn overlaid(&self, higher: &Layer) -> Layer {
        Layer {
            target: higher.target.clone().or_else(|| self.target.clone()),
            interval: higher.interval.or(self.interval),
            timeout: higher.timeout.or(self.timeout),
            fail_threshold: higher.fail_threshold.or(self.fail_threshold),
            log_file: higher.log_file.clone().or_else(|| self.log_file.clone()),
            verbose: higher.verbose.or(self.verbose),
        }
    }
}

impl From<FileConfig> for Layer {
    fn from(c: FileConfig) -> Self {
        Layer {
            target: c.target,
            interval: c.interval,
            timeout: c.timeout,
            fail_threshold: c.fail_threshold,
            log_file: c.log_file,
            verbose: c.verbose,
        }
    }
}

/// Fully resolved, validated settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub target: IpAddr,
    pub interval: Duration,
    pub timeout: Duration,
    pub fail_threshold: u32,
    pub log_file: Option<PathBuf>,
    pub verbose: bool,
}

impl Settings {
    /// Collapse the layers (lowest priority first) into validated settings.
    pub fn resolve(layers: &[Layer]) -> Result<Settings> {
        let mut acc = Layer::default();
        for layer in layers {
            acc = acc.overlaid(layer);
        }

        let target_raw = acc
            .target
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .context(
                "no probe target configured: pass --target 100.x.y.z, set TS_KEEPALIVE_TARGET, \
                 or add `target = \"100.x.y.z\"` to the config file",
            )?;
        let target: IpAddr = target_raw
            .parse()
            .with_context(|| format!("invalid target IP address: {target_raw}"))?;

        let interval = acc.interval.unwrap_or(DEFAULT_INTERVAL);
        let timeout = acc.timeout.unwrap_or(DEFAULT_TIMEOUT);
        let fail_threshold = acc.fail_threshold.unwrap_or(DEFAULT_FAIL_THRESHOLD);

        if interval == 0 {
            bail!("interval must be at least 1 second");
        }
        if timeout == 0 {
            bail!("timeout must be at least 1 second");
        }
        if timeout > interval {
            bail!("timeout ({timeout}s) must not exceed interval ({interval}s)");
        }
        if fail_threshold == 0 {
            bail!("fail-threshold must be at least 1");
        }

        Ok(Settings {
            target,
            interval: Duration::from_secs(interval),
            timeout: Duration::from_secs(timeout),
            fail_threshold,
            log_file: acc.log_file,
            verbose: acc.verbose.unwrap_or(false),
        })
    }

    /// Render the settings as the config file format, for service installation.
    pub fn to_file_config(&self) -> FileConfig {
        FileConfig {
            target: Some(self.target.to_string()),
            interval: Some(self.interval.as_secs()),
            timeout: Some(self.timeout.as_secs()),
            fail_threshold: Some(self.fail_threshold),
            log_file: self.log_file.clone(),
            verbose: Some(self.verbose),
        }
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(&self.to_file_config()).context("failed to serialize settings")
    }

    /// One-line human summary, used in logs and `config` output.
    pub fn summary(&self) -> String {
        format!(
            "target={} interval={}s timeout={}s fail_threshold={} log_file={} verbose={}",
            self.target,
            self.interval.as_secs(),
            self.timeout.as_secs(),
            self.fail_threshold,
            self.log_file
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "-".to_string()),
            self.verbose
        )
    }
}

/// Read the environment layer. Empty values are ignored.
pub fn env_layer(get: impl Fn(&str) -> Option<String>) -> Result<Layer> {
    let num = |name: &str| -> Result<Option<u64>> {
        match get(&format!("{ENV_PREFIX}{name}")) {
            Some(v) if !v.trim().is_empty() => v
                .trim()
                .parse::<u64>()
                .map(Some)
                .with_context(|| format!("invalid value for {ENV_PREFIX}{name}: {v}")),
            _ => Ok(None),
        }
    };
    let text = |name: &str| -> Option<String> {
        get(&format!("{ENV_PREFIX}{name}"))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };

    Ok(Layer {
        target: text("TARGET"),
        interval: num("INTERVAL")?,
        timeout: num("TIMEOUT")?,
        fail_threshold: num("FAIL_THRESHOLD")?.map(|v| v as u32),
        log_file: text("LOG_FILE").map(PathBuf::from),
        verbose: text("VERBOSE").map(|v| parse_bool(&v)).transpose()?,
    })
}

fn parse_bool(raw: &str) -> Result<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        other => bail!("invalid boolean value: {other}"),
    }
}

/// Load and parse a config file. A missing file is not an error: it just means
/// that layer contributes nothing.
pub fn load_file(path: &Path) -> Result<Option<FileConfig>> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let cfg = toml::from_str::<FileConfig>(&text)
                .with_context(|| format!("failed to parse config file {}", path.display()))?;
            Ok(Some(cfg))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("failed to read {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(target: &str) -> Layer {
        Layer {
            target: Some(target.to_string()),
            ..Layer::default()
        }
    }

    #[test]
    fn cli_overrides_env_overrides_file() {
        let file = Layer {
            target: Some("100.0.0.1".into()),
            interval: Some(60),
            timeout: Some(5),
            verbose: Some(false),
            ..Layer::default()
        };
        let env = Layer {
            target: Some("100.0.0.2".into()),
            interval: Some(30),
            ..Layer::default()
        };
        let cli = Layer {
            target: Some("100.0.0.3".into()),
            ..Layer::default()
        };

        let s = Settings::resolve(&[file, env, cli]).unwrap();
        assert_eq!(s.target.to_string(), "100.0.0.3");
        assert_eq!(s.interval, Duration::from_secs(30));
        assert_eq!(s.timeout, Duration::from_secs(5));
        assert!(!s.verbose);
    }

    #[test]
    fn defaults_apply_when_nothing_else_is_set() {
        let s = Settings::resolve(&[layer("100.64.1.2")]).unwrap();
        assert_eq!(s.interval, Duration::from_secs(DEFAULT_INTERVAL));
        assert_eq!(s.timeout, Duration::from_secs(DEFAULT_TIMEOUT));
        assert_eq!(s.fail_threshold, DEFAULT_FAIL_THRESHOLD);
        assert!(s.log_file.is_none());
        assert!(!s.verbose);
    }

    #[test]
    fn missing_target_is_an_error() {
        let err = Settings::resolve(&[]).unwrap_err().to_string();
        assert!(err.contains("no probe target configured"), "{err}");
    }

    #[test]
    fn blank_target_is_an_error() {
        assert!(Settings::resolve(&[layer("   ")]).is_err());
    }

    #[test]
    fn invalid_target_is_an_error() {
        let err = Settings::resolve(&[layer("not-an-ip")])
            .unwrap_err()
            .to_string();
        assert!(err.contains("invalid target"), "{err}");
    }

    #[test]
    fn ipv6_target_is_accepted() {
        let s = Settings::resolve(&[layer("fd7a:115c:a1e0::1")]).unwrap();
        assert!(s.target.is_ipv6());
    }

    #[test]
    fn rejects_degenerate_intervals() {
        let zero_interval = Layer {
            interval: Some(0),
            ..layer("100.0.0.1")
        };
        assert!(Settings::resolve(&[zero_interval]).is_err());

        let zero_timeout = Layer {
            timeout: Some(0),
            ..layer("100.0.0.1")
        };
        assert!(Settings::resolve(&[zero_timeout]).is_err());
    }

    #[test]
    fn rejects_timeout_longer_than_interval() {
        let bad = Layer {
            interval: Some(5),
            timeout: Some(10),
            ..layer("100.0.0.1")
        };
        let err = Settings::resolve(&[bad]).unwrap_err().to_string();
        assert!(err.contains("must not exceed interval"), "{err}");
    }

    #[test]
    fn env_layer_reads_all_variables() {
        let vars = [
            ("TS_KEEPALIVE_TARGET", "100.88.1.1"),
            ("TS_KEEPALIVE_INTERVAL", "15"),
            ("TS_KEEPALIVE_TIMEOUT", "1"),
            ("TS_KEEPALIVE_FAIL_THRESHOLD", "5"),
            ("TS_KEEPALIVE_LOG_FILE", "/tmp/ka.log"),
            ("TS_KEEPALIVE_VERBOSE", "yes"),
        ];
        let l = env_layer(|name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        })
        .unwrap();
        assert_eq!(l.target.as_deref(), Some("100.88.1.1"));
        assert_eq!(l.interval, Some(15));
        assert_eq!(l.timeout, Some(1));
        assert_eq!(l.fail_threshold, Some(5));
        assert_eq!(l.log_file, Some(PathBuf::from("/tmp/ka.log")));
        assert_eq!(l.verbose, Some(true));
    }

    #[test]
    fn env_layer_ignores_empty_values() {
        let l = env_layer(|_| Some("  ".to_string())).unwrap();
        assert_eq!(l, Layer::default());
    }

    #[test]
    fn env_layer_rejects_garbage() {
        let err = env_layer(|name| (name == "TS_KEEPALIVE_INTERVAL").then(|| "abc".to_string()))
            .unwrap_err()
            .to_string();
        assert!(err.contains("TS_KEEPALIVE_INTERVAL"), "{err}");
    }

    #[test]
    fn toml_roundtrip_preserves_settings() {
        let original = Settings {
            target: "100.88.132.17".parse().unwrap(),
            interval: Duration::from_secs(25),
            timeout: Duration::from_secs(2),
            fail_threshold: 3,
            log_file: Some(PathBuf::from("/var/log/ka.log")),
            verbose: true,
        };
        let text = original.to_toml().unwrap();
        let parsed: FileConfig = toml::from_str(&text).unwrap();
        let rebuilt = Settings::resolve(&[parsed.into()]).unwrap();
        assert_eq!(original, rebuilt);
    }

    #[test]
    fn unknown_config_keys_are_rejected() {
        let err = toml::from_str::<FileConfig>("target = \"100.0.0.1\"\nnope = 1\n").unwrap_err();
        assert!(err.to_string().contains("nope"), "{err}");
    }
}
