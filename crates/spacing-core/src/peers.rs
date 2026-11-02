use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub fingerprint: String,
    pub last_seen: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct PeerStore {
    #[serde(default)]
    pub peers: BTreeMap<String, PeerInfo>,
}

impl PeerStore {
    pub fn load(path: &Path) -> Result<Self, anyhow::Error> {
        match std::fs::read_to_string(path) {
            Ok(raw) => Ok(toml::from_str(&raw)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), anyhow::Error> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    /// fingerprint is the canonical "sha256:<hex>" form produced by spacing-net.
    pub fn allow(&self, fingerprint: &str) -> bool {
        self.peers.values().any(|p| p.fingerprint == fingerprint)
    }

    pub fn name_for(&self, fingerprint: &str) -> Option<&str> {
        self.peers
            .iter()
            .find(|(_, p)| p.fingerprint == fingerprint)
            .map(|(n, _)| n.as_str())
    }

    pub fn insert(&mut self, name: String, fingerprint: String) {
        self.peers.insert(
            name,
            PeerInfo {
                fingerprint,
                last_seen: None,
            },
        );
    }

    /// Refresh `last_seen` (unix seconds — avoids a datetime dep).
    pub fn seen(&mut self, name: &str) {
        if let Some(p) = self.peers.get_mut(name) {
            p.last_seen = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs().to_string())
                .ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peerstore_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("peers.toml");
        let mut s = PeerStore::default();
        s.insert("laptop".into(), "sha256:aa11".into());
        s.save(&p).unwrap();
        let s2 = PeerStore::load(&p).unwrap();
        assert!(s2.allow("sha256:aa11"));
        assert!(!s2.allow("sha256:ffff"));
        assert_eq!(s2.name_for("sha256:aa11"), Some("laptop"));
    }
}
