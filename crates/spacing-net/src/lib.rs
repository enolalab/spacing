pub mod cert;
pub mod conn;
pub mod verify;

use anyhow::{anyhow, Context};
use spacing_core::peers::PeerStore;
use spacing_core::protocol::{Message, PROTOCOL_VERSION};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::{TcpStream, ToSocketAddrs};
use tokio_rustls::rustls::{ClientConfig, ServerConfig};
use tokio_rustls::{TlsAcceptor, TlsConnector, TlsStream};

pub type Rd = ReadHalf<TlsStream<TcpStream>>;
pub type Wr = WriteHalf<TlsStream<TcpStream>>;

pub struct ServerConn {
    pub rd: Rd,
    pub wr: Wr,
    pub peer_fp: String,
    pub peer_name: String,
}

pub struct ClientConn {
    pub rd_half: Rd,
    pub wr_half: Wr,
    pub peer_fp: String,
}

fn ensure_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .ok();
    });
}

pub async fn tcp_connect<A: ToSocketAddrs>(addr: A) -> TcpStream {
    TcpStream::connect(addr).await.expect("tcp connect")
}

fn peer_fp(stream: &TlsStream<TcpStream>) -> anyhow::Result<String> {
    let (_, common) = stream.get_ref();
    let certs = common
        .peer_certificates()
        .ok_or_else(|| anyhow!("peer presented no certificate"))?;
    Ok(cert::fingerprint_of(&certs[0]))
}

/// Server side: accept TCP, run mutual-TLS, verify peer fingerprint.
/// `peers` is SHARED with the server loop — a successful pair must be
/// visible to subsequent accepts without a restart.
/// `pairing_open`: one-shot — unknown fingerprints are accepted at TLS
/// layer; caller must treat next message as PairRequest.
pub async fn accept_server(
    tcp: TcpStream,
    id: cert::Identity,
    peers: Arc<RwLock<PeerStore>>,
    pairing_open: Arc<AtomicBool>,
    peers_path: std::path::PathBuf,
) -> anyhow::Result<ServerConn> {
    ensure_provider();
    // Input frames are tiny and latency-critical — Nagle's algorithm would
    // hold them for a delayed ACK (~40ms on Windows).
    tcp.set_nodelay(true)?;
    let verifier = Arc::new(verify::PairingVerifier {
        peers: peers.clone(),
        pairing_open: pairing_open.clone(),
    });
    let cfg = ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(id.certs.clone(), id.key.clone_key())
        .context("server tls config")?;
    let stream: TlsStream<_> = TlsAcceptor::from(Arc::new(cfg))
        .accept(tcp)
        .await?
        .into();
    let peer_fp = peer_fp(&stream)?;
    let paired = peers.read().unwrap().allow(&peer_fp);
    if !paired && !pairing_open.load(Ordering::Relaxed) {
        anyhow::bail!("untrusted fingerprint {peer_fp}");
    }
    let (mut rd, mut wr) = tokio::io::split(stream);
    // First frame: Hello (paired peer) or PairRequest (new peer).
    let msg = conn::recv_msg(&mut rd).await?;
    let peer_name = match msg {
        Message::Hello { version, hostname } => {
            anyhow::ensure!(version == PROTOCOL_VERSION, "version mismatch {version}");
            anyhow::ensure!(paired, "hello from unpaired client");
            peers.write().unwrap().seen(&hostname);
            conn::send_msg(
                &mut wr,
                &Message::HelloAck {
                    version: PROTOCOL_VERSION,
                },
            )
            .await?;
            hostname
        }
        Message::PairRequest { hostname } => {
            let accepted = !paired && {
                // pairing_open was consumed by the TLS verifier to let
                // this unknown cert through — grant exactly once.
                pairing_open.swap(false, Ordering::Relaxed)
            };
            if accepted {
                {
                    let mut g = peers.write().unwrap();
                    g.insert(hostname.clone(), peer_fp.clone());
                    g.save(&peers_path)?; // persist so future connects verify
                }
                conn::send_msg(&mut wr, &Message::PairAccept).await?;
            } else {
                conn::send_msg(&mut wr, &Message::PairReject).await?;
                anyhow::bail!("pairing rejected (not in --pairing mode or already paired)");
            }
            hostname
        }
        other => anyhow::bail!("unexpected first message {other:?}"),
    };
    Ok(ServerConn {
        rd,
        wr,
        peer_fp,
        peer_name,
    })
}

/// Client side: connect TCP, mutual-TLS. `verify` checks the server
/// fingerprint — pass `|fp| peers.allow(fp)` normally, `|_| true` when
/// pairing (fp is reported to the caller for the trust prompt).
pub async fn connect_client(
    tcp: TcpStream,
    server_name: &str,
    id: cert::Identity,
    verify: impl Fn(&str) -> bool,
) -> anyhow::Result<ClientConn> {
    ensure_provider();
    tcp.set_nodelay(true)?;
    let cfg = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verify::AcceptAnyServerCert))
        .with_client_auth_cert(id.certs.clone(), id.key.clone_key())
        .context("client tls config")?;
    let name = rustls::pki_types::ServerName::try_from(server_name.to_string())
        .map_err(|_| anyhow!("invalid server name {server_name}"))?;
    let stream: TlsStream<_> = TlsConnector::from(Arc::new(cfg))
        .connect(name, tcp)
        .await?
        .into();
    let peer_fp = peer_fp(&stream)?;
    anyhow::ensure!(
        verify(&peer_fp),
        "server fingerprint {peer_fp} not in peers.toml — run `spacing pair` first"
    );
    let (rd_half, wr_half) = tokio::io::split(stream);
    Ok(ClientConn {
        rd_half,
        wr_half,
        peer_fp,
    })
}
