use anyhow::Context;
use spacing_core::config::{load_config, Mode};
use spacing_core::peers::PeerStore;
use spacing_core::protocol::{Message, PROTOCOL_VERSION};
use spacing_inject::UinputInject;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const WATCHDOG_MS: u64 = 500;

pub async fn run(dir: PathBuf, to: Option<String>, cfg_path: Option<PathBuf>) -> anyhow::Result<()> {
    let cfg_path = cfg_path.unwrap_or_else(|| dir.join("config.toml"));
    let cfg = load_config(&cfg_path).ok(); // optional in client mode
    let addr = to
        .or_else(|| cfg.as_ref().and_then(|c| c.connect_to.clone()))
        .context("no server address: pass --to host:port or set connect_to")?;
    if let Some(c) = &cfg {
        anyhow::ensure!(c.mode == Mode::Client, "config mode must be \"client\"");
    }
    let peers = PeerStore::load(&dir.join("peers.toml"))?;
    let id = spacing_net::cert::ensure_identity(&dir)?;
    let tcp = tokio::net::TcpStream::connect(&addr).await?;
    let host = addr.split(':').next().unwrap_or("server");
    let conn = spacing_net::connect_client(tcp, host, id, |fp| peers.allow(fp))
        .await
        .context("TLS connect/fingerprint check failed — run `spacing pair` first")?;
    let (mut rd, mut wr) = (conn.rd_half, conn.wr_half);
    tracing::info!(server = %addr, fp = %conn.peer_fp, "connected");

    let hostname = gethostname::gethostname().to_string_lossy().into_owned();
    spacing_net::conn::send_msg(
        &mut wr,
        &Message::Hello {
            version: PROTOCOL_VERSION,
            hostname,
        },
    )
    .await?;

    let mut inject = UinputInject::create()
        .context("cannot open /dev/uinput — modprobe uinput + apply udev rule in README")?;
    let mut last_rx = Instant::now();
    let mut released = true;
    let mut tick = tokio::time::interval(Duration::from_millis(100));

    loop {
        tokio::select! {
            m = spacing_net::conn::recv_msg(&mut rd) => {
                match m {
                    Ok(Message::Input(ev)) => {
                        match inject.inject(&ev) {
                            Ok(()) => {
                                last_rx = Instant::now();
                                released = false;
                            }
                            Err(e) => {
                                // spec §5: client báo Error → server về Local;
                                // release phím đang giữ trước khi chết.
                                let _ = spacing_net::conn::send_msg(&mut wr, &Message::Error {
                                    code: 1,
                                    message: e.to_string(),
                                })
                                .await;
                                let _ = inject.release_all();
                                anyhow::bail!("inject failed: {e}");
                            }
                        }
                    }
                    Ok(Message::Heartbeat) => {
                        last_rx = Instant::now();
                    }
                    Ok(Message::ReleaseControl) => {
                        inject.release_all()?;
                        last_rx = Instant::now();
                        released = true;
                    }
                    Ok(Message::HelloAck { .. }) => {
                        last_rx = Instant::now();
                    }
                    Ok(Message::PairAccept | Message::PairReject) => {}
                    Ok(Message::Error { message, .. }) => {
                        tracing::warn!("server error: {message}")
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::error!("connection lost: {e}");
                        inject.release_all()?;
                        anyhow::bail!("connection lost: {e}");
                    }
                }
            }
            _ = tick.tick() => {
                if !released && last_rx.elapsed() > Duration::from_millis(WATCHDOG_MS) {
                    tracing::warn!(
                        "watchdog: no input/heartbeat for {WATCHDOG_MS}ms — releasing keys"
                    );
                    inject.release_all()?;
                    released = true;
                }
            }
        }
    }
}
