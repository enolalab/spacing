use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Server,
    Client,
}

/// Input backend selection. `auto` picks the portal backend on Wayland
/// when the desktop supports it, otherwise evdev/uinput.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    #[default]
    Auto,
    Evdev,
    Portal,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub mode: Mode,
    #[serde(default = "default_bind")]
    pub bind: String,
    /// Client mode only.
    pub connect_to: Option<String>,
    #[serde(default)]
    pub backend: Backend,
    #[serde(default)]
    pub server: ServerCfg,
}

fn default_bind() -> String {
    "0.0.0.0:24800".into()
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ServerCfg {
    #[serde(default)]
    pub hotkeys: HotkeysCfg,
    #[serde(default)]
    pub layout: LayoutCfg,
}

/// Screen-edge → client mapping for portal barrier switching:
/// `[server.layout.edges] right = "laptop"`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LayoutCfg {
    #[serde(default)]
    pub edges: BTreeMap<String, String>,
}

/// `local` is the switch-back combo; every other entry is
/// `"Combo" = "client-name"`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HotkeysCfg {
    pub local: Option<String>,
    #[serde(flatten)]
    pub switch: BTreeMap<String, String>,
}

pub fn load_config(path: &Path) -> Result<Config, anyhow::Error> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
    let cfg: Config = toml::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: &str = r#"
        mode = "server"
        bind = "0.0.0.0:24800"
        [server.hotkeys]
        local = "Ctrl+Alt+0"
        "Ctrl+Alt+1" = "laptop"
        "Ctrl+Alt+2" = "desktop"
    "#;

    #[test]
    fn parses_server_config() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.toml");
        std::fs::write(&p, CFG).unwrap();
        let c = load_config(&p).unwrap();
        assert_eq!(c.mode, Mode::Server);
        assert_eq!(c.bind, "0.0.0.0:24800");
        assert_eq!(c.backend, Backend::Auto);
        assert_eq!(c.server.hotkeys.local.as_deref(), Some("Ctrl+Alt+0"));
        assert_eq!(c.server.hotkeys.switch["Ctrl+Alt+1"], "laptop");
        assert_eq!(c.server.hotkeys.switch["Ctrl+Alt+2"], "desktop");
    }

    #[test]
    fn parses_backend_selection() {
        for (raw, want) in [
            ("evdev", Backend::Evdev),
            ("portal", Backend::Portal),
            ("auto", Backend::Auto),
        ] {
            let cfg: Config = toml::from_str(&format!(
                "mode = \"server\"\nbackend = \"{raw}\"\n"
            ))
            .unwrap();
            assert_eq!(cfg.backend, want, "{raw}");
        }
    }

    #[test]
    fn parses_layout_edges() {
        let cfg: Config = toml::from_str(
            "mode = \"server\"\n[server.layout.edges]\nright = \"laptop\"\nleft = \"desktop\"\n",
        )
        .unwrap();
        assert_eq!(cfg.server.layout.edges["right"], "laptop");
        assert_eq!(cfg.server.layout.edges["left"], "desktop");
        assert!(!cfg.server.layout.edges.contains_key("top"));
    }
}
