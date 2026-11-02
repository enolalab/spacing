use anyhow::Context;
use spacing_capture::{select_capture, CaptureBackend, CaptureEvent};
use spacing_core::clipdedup::ClipDedup;
use spacing_core::combo::parse_combo;
use spacing_core::config::{load_config, Mode};
use spacing_core::peers::PeerStore;
use spacing_core::protocol::{Message, CLIPBOARD_MAX};
use spacing_core::router::{Decision, Router, Target};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

const SEND_TIMEOUT: Duration = Duration::from_millis(20);
const HEARTBEAT_MS: u64 = 250;
const CLIPBOARD_POLL_MS: u64 = 300;

/// Reader tasks report client deaths AND inbound clipboard updates.
enum Ctrl {
    Dead(String),
    Clip(String),
}

async fn writer_task(mut wr: spacing_net::Wr, mut rx: mpsc::Receiver<Message>) {
    while let Some(m) = rx.recv().await {
        if spacing_net::conn::send_msg(&mut wr, &m).await.is_err() {
            break;
        }
    }
}

/// Reads client->server control messages; reports peer_name on EOF/Error.
async fn reader_task(mut rd: spacing_net::Rd, name: String, ctrl: mpsc::Sender<Ctrl>) {
    loop {
        match spacing_net::conn::recv_msg(&mut rd).await {
            Ok(Message::Error { message, .. }) => {
                tracing::warn!(peer = %name, "client error: {message}");
                let _ = ctrl.send(Ctrl::Dead(name)).await;
                return;
            }
            Ok(Message::ClipboardSet { text }) => {
                if text.len() <= CLIPBOARD_MAX {
                    let _ = ctrl.send(Ctrl::Clip(text)).await;
                } else {
                    tracing::warn!(peer = %name, "clipboard >1MiB — dropped");
                }
            }
            Ok(_) => {}
            Err(_) => {
                let _ = ctrl.send(Ctrl::Dead(name)).await;
                return;
            }
        }
    }
}

pub async fn run(dir: PathBuf, cfg_path: Option<PathBuf>, pairing: bool) -> anyhow::Result<()> {
    let cfg_path = cfg_path.unwrap_or_else(|| dir.join("config.toml"));
    let cfg = load_config(&cfg_path)?;
    anyhow::ensure!(cfg.mode == Mode::Server, "config mode must be \"server\"");
    // Shared with every accept task — pairing inserts are visible immediately.
    let peers = Arc::new(RwLock::new(PeerStore::load(&dir.join("peers.toml"))?));
    let id = spacing_net::cert::ensure_identity(&dir)?;
    tracing::info!(fp = %id.fingerprint, "identity loaded");

    let local = parse_combo(
        cfg.server
            .hotkeys
            .local
            .as_deref()
            .unwrap_or("Ctrl+Alt+0"),
    )?;
    let mut switch = Vec::new();
    for (combo_s, name) in &cfg.server.hotkeys.switch {
        switch.push((parse_combo(combo_s)?, name.clone()));
    }
    let mut router = Router::new(local, switch);

    let mut cap = select_capture(cfg.backend).context(
        "cannot initialize input capture — check backend config, input group, or the udev rule in README",
    )?;

    let listener = TcpListener::bind(&cfg.bind).await?;
    tracing::info!(bind = %cfg.bind, pairing, "listening");

    // peer_name -> channel to that client's writer task
    let mut conns: HashMap<String, mpsc::Sender<Message>> = HashMap::new();
    let pairing_open = Arc::new(AtomicBool::new(pairing));
    // accepted conns arrive here from accept tasks
    let (new_tx, mut new_rx) = mpsc::channel::<spacing_net::ServerConn>(8);
    // dead/errored peer_names arrive here from reader tasks
    let (ctrl_tx, mut ctrl_rx) = mpsc::channel::<Ctrl>(8);
    let mut hb = tokio::time::interval(Duration::from_millis(HEARTBEAT_MS));
    hb.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // latest-wins text clipboard with echo suppression
    let mut clip = spacing_inject::clipboard::select_clipboard();
    let mut clip_dedup = ClipDedup::default();
    let mut clip_tick = tokio::time::interval(Duration::from_millis(CLIPBOARD_POLL_MS));
    clip_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            acc = listener.accept() => {
                let (tcp, _) = acc?;
                let (idc, pc, po, ntx, pp) = (
                    id.clone(),
                    peers.clone(),
                    pairing_open.clone(),
                    new_tx.clone(),
                    dir.join("peers.toml"),
                );
                tokio::spawn(async move {
                    match spacing_net::accept_server(tcp, idc, pc, po, pp).await {
                        Ok(conn) => {
                            tracing::info!(peer = %conn.peer_name, fp = %conn.peer_fp, "client connected");
                            let _ = ntx.send(conn).await;
                        }
                        Err(e) => tracing::warn!("connection rejected: {e}"),
                    }
                });
            }
            Some(conn) = new_rx.recv() => {
                let name = conn.peer_name.clone();
                if conns.contains_key(&name) {
                    tracing::warn!(peer = %name, "duplicate peer name — replacing connection");
                }
                let (tx, rx) = mpsc::channel::<Message>(256);
                conns.insert(name.clone(), tx);
                tokio::spawn(writer_task(conn.wr, rx));
                tokio::spawn(reader_task(conn.rd, name, ctrl_tx.clone()));
            }
            Some(ctrl) = ctrl_rx.recv() => match ctrl {
                Ctrl::Dead(dead) => {
                    tracing::warn!(peer = %dead, "client lost");
                    let prev = router.target().clone();
                    conns.remove(&dead);
                    if matches!(&prev, Target::Remote(n) if n == &dead) {
                        to_local(&mut *cap, &mut router, &conns, &prev).await;
                    }
                }
                Ctrl::Clip(text) => {
                    clip_dedup.applied_remote(&text);
                    if let Err(e) = clip.set_text(&text) {
                        tracing::warn!("clipboard set failed: {e}");
                    }
                }
            },
            // cap.next_event() borrows `cap` only while pending — arms can
            // grab()/ungrab() freely after it resolves.
            Some(cev) = cap.next_event() => {
                let ev = match cev {
                    CaptureEvent::Input(ev) => ev,
                    // Portal edge events — wired to the router in T6.
                    CaptureEvent::Activated { ref zone } => {
                        tracing::info!(%zone, "capture barrier activated");
                        continue;
                    }
                    CaptureEvent::Deactivated => {
                        tracing::info!("capture deactivated");
                        continue;
                    }
                };
                let prev = router.target().clone();
                match router.on_event(ev) {
                    Decision::Drop => {}
                    Decision::Forward(e) => {
                        if let Target::Remote(name) = router.target() {
                            if let Some(tx) = conns.get(name) {
                                if tx.send_timeout(Message::Input(e), SEND_TIMEOUT).await.is_err() {
                                    fail_to_local(&mut *cap, &mut router).await;
                                }
                            }
                        }
                    }
                    Decision::SwitchToRemote(name) => {
                        if !conns.contains_key(&name) {
                            // hotkey targets a peer that isn't connected —
                            // grabbing here would send input into the void.
                            tracing::warn!(target = %name, "peer not connected — staying local");
                            router.on_disconnected();
                        } else {
                            tracing::info!(target = %name, "switching to remote");
                            // remote->remote: tell the old client to release held keys
                            if let Target::Remote(old) = &prev {
                                if old != &name {
                                    if let Some(tx) = conns.get(old) {
                                        let _ = tx
                                            .send_timeout(Message::ReleaseControl, SEND_TIMEOUT)
                                            .await;
                                    }
                                }
                            }
                            if let Err(e) = cap.grab().await {
                                tracing::error!("grab failed: {e} — staying local");
                                router.on_disconnected();
                            } else {
                                // spec: push local clipboard on switch
                                if let Ok(cur) = clip.get_text() {
                                    let _ = clip_dedup.poll(cur.as_deref()); // mark seen
                                    if let Some(text) = cur.filter(|t| t.len() <= CLIPBOARD_MAX) {
                                        if let Some(tx) = conns.get(&name) {
                                            let _ = tx
                                                .send_timeout(
                                                    Message::ClipboardSet { text },
                                                    SEND_TIMEOUT,
                                                )
                                                .await;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Decision::SwitchToLocal => {
                        tracing::info!("back to local");
                        to_local(&mut *cap, &mut router, &conns, &prev).await;
                    }
                }
            }
            _ = hb.tick(), if matches!(router.target(), Target::Remote(_)) => {
                if let Target::Remote(name) = router.target() {
                    if let Some(tx) = conns.get(name) {
                        if tx.send_timeout(Message::Heartbeat, SEND_TIMEOUT).await.is_err() {
                            fail_to_local(&mut *cap, &mut router).await;
                        }
                    }
                }
            }
            // watch local clipboard only while remote — on-switch push
            // covers copies made while local.
            _ = clip_tick.tick(), if matches!(router.target(), Target::Remote(_)) => {
                let cur = clip.get_text().unwrap_or_default();
                if let Some(text) = clip_dedup.poll(cur.as_deref()) {
                    if text.len() <= CLIPBOARD_MAX {
                        if let Target::Remote(name) = router.target() {
                            if let Some(tx) = conns.get(name) {
                                // dropped clipboard is not a session failure
                                let _ = tx
                                    .send_timeout(Message::ClipboardSet { text }, SEND_TIMEOUT)
                                    .await;
                            }
                        }
                    } else {
                        tracing::warn!("clipboard >1MiB — not forwarded");
                    }
                }
            }
        }
    }
}

/// Spec order: ReleaseControl → (client release_all) → ungrab → Local.
/// `prev` is the target we just left — only it needs ReleaseControl.
async fn to_local(
    cap: &mut dyn CaptureBackend,
    router: &mut Router,
    conns: &HashMap<String, mpsc::Sender<Message>>,
    prev: &Target,
) {
    router.on_disconnected();
    if let Target::Remote(old) = prev {
        if let Some(tx) = conns.get(old) {
            let _ = tx.send_timeout(Message::ReleaseControl, SEND_TIMEOUT).await;
        }
    }
    let _ = cap.ungrab().await;
}

async fn fail_to_local(cap: &mut dyn CaptureBackend, router: &mut Router) {
    tracing::warn!("send failed — returning to local");
    router.on_disconnected();
    let _ = cap.ungrab().await;
}
