use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub editor: EditorConfig,
    #[serde(default)]
    pub resources: ResourceConfig,
    #[serde(default)]
    pub lsp: LspConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UiConfig {
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_true")]
    pub minimize_draws: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EditorConfig {
    #[serde(default = "default_tab_size")]
    pub tab_size: usize,
    #[serde(default = "default_true")]
    pub lazy_tabs: bool,
    #[serde(default = "default_idle")]
    pub tab_unload_idle_secs: u64,
    #[serde(default)]
    pub recent_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResourceConfig {
    #[serde(default = "default_true")]
    pub enable_monitor: bool,
    #[serde(default = "default_throttle")]
    pub index_throttle_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LspConfig {
    #[serde(default = "default_true")]
    pub on_demand: bool,
    #[serde(default = "default_idle")]
    pub idle_shutdown_secs: u64,
    #[serde(default)]
    pub servers: HashMap<String, LanguageServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

fn default_theme() -> String {
    "dark".to_string()
}
fn default_true() -> bool {
    true
}
fn default_tab_size() -> usize {
    4
}
fn default_idle() -> u64 {
    300
}
fn default_throttle() -> u64 {
    50
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ui: UiConfig::default(),
            editor: EditorConfig::default(),
            resources: ResourceConfig::default(),
            lsp: LspConfig::default(),
        }
    }
}

impl Config {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        if !path.as_ref().exists() {
            let cfg = Self::default();
            let s = toml::to_string_pretty(&cfg)?;
            fs::write(path, s)?;
            return Ok(cfg);
        }
        let s = fs::read_to_string(path)?;
        Ok(toml::from_str(&s)?)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let serialized = toml::to_string_pretty(self)?;
        fs::write(path, serialized)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Config;
    use std::path::PathBuf;

    #[test]
    fn existing_lsp_config_loads_without_server_entries() {
        let config: Config = toml::from_str("[lsp]\non_demand = true\n").unwrap();
        assert!(config.lsp.on_demand);
        assert!(config.lsp.servers.is_empty());
    }

    #[test]
    fn language_server_commands_load_from_toml() {
        let config: Config = toml::from_str(
            "[lsp.servers.rust]\ncommand = \"rust-analyzer\"\nargs = [\"--log-file\", \"server.log\"]\n",
        )
        .unwrap();
        let rust = config.lsp.servers.get("rust").unwrap();
        assert_eq!(rust.command, "rust-analyzer");
        assert_eq!(rust.args, ["--log-file", "server.log"]);
    }

    #[test]
    fn recent_files_round_trip_through_config() {
        let config: Config =
            toml::from_str(" [editor]\nrecent_files = [\"/workspace/main.rs\"]\n").unwrap();
        assert_eq!(
            config.editor.recent_files,
            vec![PathBuf::from("/workspace/main.rs")]
        );
    }
}
