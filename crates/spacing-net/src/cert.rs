use anyhow::Context;
use sha2::{Digest, Sha256};
use std::path::Path;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// `PrivateKeyDer` does NOT implement `Clone` (only `clone_key()`),
/// so a manual impl is required — `#[derive(Clone)]` will not compile.
pub struct Identity {
    pub certs: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
    pub fingerprint: String,
}

impl Clone for Identity {
    fn clone(&self) -> Self {
        Self {
            certs: self.certs.clone(),
            key: self.key.clone_key(),
            fingerprint: self.fingerprint.clone(),
        }
    }
}

/// Load or generate a self-signed identity at `<dir>/{cert,key}.pem`.
pub fn ensure_identity(dir: &Path) -> anyhow::Result<Identity> {
    std::fs::create_dir_all(dir)?;
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    if !cert_path.exists() || !key_path.exists() {
        let hostname = gethostname::gethostname().to_string_lossy().into_owned();
        let rcgen::CertifiedKey { cert, key_pair } =
            rcgen::generate_simple_self_signed(vec![hostname])?;
        std::fs::write(&cert_path, cert.pem())?;
        std::fs::write(&key_path, key_pair.serialize_pem())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    let mut cr = std::io::BufReader::new(std::fs::File::open(&cert_path)?);
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cr)
        .collect::<Result<_, _>>()
        .context("parse cert.pem")?;
    let mut kr = std::io::BufReader::new(std::fs::File::open(&key_path)?);
    let key = rustls_pemfile::private_key(&mut kr)?
        .ok_or_else(|| anyhow::anyhow!("no private key in key.pem"))?;
    let fingerprint = fingerprint_of(&certs[0]);
    Ok(Identity {
        certs,
        key,
        fingerprint,
    })
}

/// Canonical "sha256:<lowercase hex>" fingerprint of a DER certificate.
pub fn fingerprint_of(der: &CertificateDer<'_>) -> String {
    let digest = Sha256::digest(der.as_ref());
    let mut s = String::with_capacity(71);
    s.push_str("sha256:");
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
