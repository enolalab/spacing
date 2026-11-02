use anyhow::Context;
use spacing_core::clipdedup::ClipDedup;
use spacing_core::config::{load_config, Mode};
use spacing_core::peers::PeerStore;
use spacing_core::protocol::{Message, CLIPBOARD_MAX, PROTOCOL_VERSION};
use spacing_inject::{select_inject, InjectionBackend};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const WATCHDOG_MS: u64 = 500;
const CLIPBOARD_POLL_MS: u64 = 300;

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

    let backend = cfg.map(|c| c.backend).unwrap_or_default();
    let mut inject = select_inject(backend)
        .await
        .context("cannot initialize input injection — check backend config or the udev rule in README")?;
    let mut last_rx = Instant::now();
    let mut released = true;
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    // local clipboard watch → ClipboardSet upstream; echo-suppressed
    let mut clip = spacing_inject::clipboard::select_clipboard();
    let mut clip_dedup = ClipDedup::default();
    let mut clip_tick = tokio::time::interval(Duration::from_millis(CLIPBOARD_POLL_MS));
    clip_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Cancellation-safe read buffer: `read_buf` appends into `netbuf`, so
    // bytes already received survive a `select!` cancellation — a
    // `recv_msg` future here would lose partially-read bytes and desync
    // the frame stream (observed as intermittent "frame too large").
    let mut netbuf: Vec<u8> = Vec::with_capacity(16 * 1024);

    loop {
        tokio::select! {
            r = tokio::io::AsyncReadExt::read_buf(&mut rd, &mut netbuf) => {
                match r {
                    Ok(0) => {
                        tracing::error!("connection lost: server closed");
                        inject.release_all().await?;
                        anyhow::bail!("connection lost: server closed");
                    }
                    Ok(_) => {
                        loop {
                            match spacing_net::conn::try_decode(&mut netbuf) {
                                Ok(Some(m)) => {
                                    match handle_msg(&m, &mut *inject, &mut *clip, &mut clip_dedup, &mut wr, &mut last_rx, &mut released).await {
                                        Ok(()) => {}
                                        Err(e) => return Err(e),
                                    }
                                }
                                Ok(None) => break,
                                Err(e) => {
                                    tracing::error!("connection lost: {e}");
                                    let _ = inject.release_all().await;
                                    anyhow::bail!("connection lost: {e}");
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("connection lost: {e}");
                        let _ = inject.release_all().await;
                        anyhow::bail!("connection lost: {e}");
                    }
                }
            }
            _ = tick.tick() => {
                if !released && last_rx.elapsed() > Duration::from_millis(WATCHDOG_MS) {
                    tracing::warn!(
                        "watchdog: no input/heartbeat for {WATCHDOG_MS}ms — releasing keys"
                    );
                    inject.release_all().await?;
                    released = true;
                }
            }
            _ = clip_tick.tick() => {
                let cur = clip.get_text().unwrap_or_default();
                if let Some(text) = clip_dedup.poll(cur.as_deref()) {
                    if text.len() <= CLIPBOARD_MAX {
                        if spacing_net::conn::send_msg(&mut wr, &Message::ClipboardSet { text })
                            .await
                            .is_err()
                        {
                            let _ = inject.release_all().await;
                            anyhow::bail!("connection lost: clipboard send failed");
                        }
                    } else {
                        tracing::warn!("clipboard >1MiB — not forwarded");
                    }
                }
            }
        }
    }
}

async fn handle_msg(
    m: &Message,
    inject: &mut dyn InjectionBackend,
    clip: &mut dyn spacing_inject::clipboard::Clipboard,
    clip_dedup: &mut ClipDedup,
    wr: &mut spacing_net::Wr,
    last_rx: &mut Instant,
    released: &mut bool,
) -> anyhow::Result<()> {
    match m {
        Message::Input(ev) => match inject.inject(ev).await {
            Ok(()) => {
                *last_rx = Instant::now();
                *released = false;
            }
            Err(e) => {
                // spec §5: client báo Error → server về Local;
                // release phím đang giữ trước khi chết.
                let _ = spacing_net::conn::send_msg(wr, &Message::Error {
                    code: 1,
                    message: e.to_string(),
                })
                .await;
                let _ = inject.release_all().await;
                anyhow::bail!("inject failed: {e}");
            }
        },
        Message::Heartbeat | Message::HelloAck { .. } => {
            *last_rx = Instant::now();
        }
        Message::ReleaseControl => {
            inject.release_all().await?;
            *last_rx = Instant::now();
            *released = true;
        }
        Message::ClipboardSet { text } => {
            *last_rx = Instant::now();
            if text.len() <= CLIPBOARD_MAX {
                clip_dedup.applied_remote(text);
                if let Err(e) = clip.set_text(text) {
                    tracing::warn!("clipboard set failed: {e}");
                }
            } else {
                let _ = spacing_net::conn::send_msg(wr, &Message::Error {
                    code: 2,
                    message: "clipboard exceeds 1MiB".into(),
                })
                .await;
            }
        }
        Message::PairAccept | Message::PairReject => {}
        Message::Error { message, .. } => {
            tracing::warn!("server error: {message}")
        }
        _ => {}
    }
    Ok(())
}
