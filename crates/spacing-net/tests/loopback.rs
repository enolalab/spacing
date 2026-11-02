use spacing_core::peers::PeerStore;
use spacing_core::protocol::Message;
use spacing_net::*;
use tokio::net::TcpListener;

#[tokio::test]
async fn paired_handshake_and_echo() {
    let dir = tempfile::tempdir().unwrap();
    let server_id = cert::ensure_identity(&dir.path().join("srv")).unwrap();
    let client_id = cert::ensure_identity(&dir.path().join("cli")).unwrap();

    // client trusts server fp; server trusts client fp
    let mut client_peers = PeerStore::default();
    client_peers.insert("srv".into(), server_id.fingerprint.clone());
    let mut server_peers = PeerStore::default();
    server_peers.insert("cli".into(), client_id.fingerprint.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let srv_peers = std::sync::Arc::new(std::sync::RwLock::new(server_peers.clone()));
    let pairing_open = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let peers_path = dir.path().join("peers.toml");

    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        accept_server(tcp, server_id.clone(), srv_peers, pairing_open, peers_path)
            .await
            .unwrap()
    });

    let mut conn = connect_client(
        tcp_connect(addr).await,
        "localhost",
        client_id.clone(),
        move |fp| client_peers.allow(fp),
    )
    .await
    .unwrap();
    conn::send_msg(
        &mut conn.wr_half,
        &Message::Hello {
            version: spacing_core::protocol::PROTOCOL_VERSION,
            hostname: "cli".into(),
        },
    )
    .await
    .unwrap();

    let mut server_conn = server.await.unwrap();
    conn::send_msg(&mut server_conn.wr, &Message::Heartbeat)
        .await
        .unwrap();
    let m = conn::recv_msg(&mut conn.rd_half).await.unwrap();
    // first frame back is HelloAck; read again for Heartbeat
    assert!(matches!(m, Message::HelloAck { .. }) || matches!(m, Message::Heartbeat));
}

/// Pairing window: unknown client cert accepted at TLS, PairRequest →
/// PairAccept, fingerprint persisted to peers.toml, window consumed.
#[tokio::test]
async fn pairing_window_registers_client_once() {
    use std::sync::atomic::Ordering;

    let dir = tempfile::tempdir().unwrap();
    let server_id = cert::ensure_identity(&dir.path().join("srv")).unwrap();
    let client_id = cert::ensure_identity(&dir.path().join("cli")).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let peers = std::sync::Arc::new(std::sync::RwLock::new(PeerStore::default()));
    let pairing_open = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let peers_path = dir.path().join("peers.toml");

    let (srv_peers, po) = (peers.clone(), pairing_open.clone());
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        accept_server(tcp, server_id, srv_peers, po, peers_path)
            .await
            .unwrap()
    });

    let mut conn = connect_client(
        tcp_connect(addr).await,
        "localhost",
        client_id.clone(),
        |_| true,
    )
    .await
    .unwrap();
    conn::send_msg(
        &mut conn.wr_half,
        &Message::PairRequest {
            hostname: "cli".into(),
        },
    )
    .await
    .unwrap();
    match conn::recv_msg(&mut conn.rd_half).await.unwrap() {
        Message::PairAccept => {}
        other => panic!("expected PairAccept, got {other:?}"),
    }
    server.await.unwrap();

    // in-memory store updated (shared Arc), persisted to disk, window closed
    assert!(peers.read().unwrap().allow(&client_id.fingerprint));
    assert!(!pairing_open.load(Ordering::Relaxed));
    let reloaded = PeerStore::load(&dir.path().join("peers.toml")).unwrap();
    assert!(reloaded.allow(&client_id.fingerprint));
}

/// Stress: mimic the server's writer_task (mpsc -> send_msg) pushing Input
/// events at high rate interleaved with Heartbeats. Client must decode
/// every frame cleanly — no "frame too large" / misalignment.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stress_framed_stream_no_desync() {
    use spacing_core::event::InputEvent;

    let dir = tempfile::tempdir().unwrap();
    let server_id = cert::ensure_identity(&dir.path().join("srv")).unwrap();
    let client_id = cert::ensure_identity(&dir.path().join("cli")).unwrap();

    let mut client_peers = PeerStore::default();
    client_peers.insert("srv".into(), server_id.fingerprint.clone());
    let mut server_peers = PeerStore::default();
    server_peers.insert("cli".into(), client_id.fingerprint.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let srv_peers = std::sync::Arc::new(std::sync::RwLock::new(server_peers.clone()));
    let pairing_open = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let peers_path = dir.path().join("peers.toml");

    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        accept_server(tcp, server_id.clone(), srv_peers, pairing_open, peers_path)
            .await
            .unwrap()
    });

    let mut conn = connect_client(
        tcp_connect(addr).await,
        "localhost",
        client_id.clone(),
        move |fp| client_peers.allow(fp),
    )
    .await
    .unwrap();
    conn::send_msg(
        &mut conn.wr_half,
        &Message::Hello {
            version: spacing_core::protocol::PROTOCOL_VERSION,
            hostname: "cli".into(),
        },
    )
    .await
    .unwrap();
    let server_conn = server.await.unwrap();

    // server side: exactly like writer_task — mpsc -> send_msg
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Message>(256);
    tokio::spawn(async move {
        let mut wr = server_conn.wr;
        while let Some(m) = rx.recv().await {
            if conn::send_msg(&mut wr, &m).await.is_err() {
                break;
            }
        }
    });
    // reader for client->server (unused traffic)
    tokio::spawn(async move {
        let mut rd = server_conn.rd;
        while conn::recv_msg(&mut rd).await.is_ok() {}
    });

    // pump: alternating Input + every 5th Heartbeat, like serve's select
    const N: u32 = 20_000;
    let feeder = tokio::spawn(async move {
        for i in 0..N {
            let m = if i % 5 == 0 {
                Message::Heartbeat
            } else {
                Message::Input(InputEvent::KeyDown(0x04))
            };
            if tx.send(m).await.is_err() {
                return;
            }
        }
    });

    let mut ok = 0u32;
    let mut read = conn.rd_half;
    // skip HelloAck (first frame)
    let _ = conn::recv_msg(&mut read).await.unwrap();
    while ok < N {
        match conn::recv_msg(&mut read).await {
            Ok(Message::Input(_) | Message::Heartbeat) => ok += 1,
            Ok(other) => panic!("unexpected {other:?} at {ok}"),
            Err(e) => panic!("desync at msg {ok}: {e}"),
        }
    }
    feeder.await.unwrap();
}

/// Regression: the real client reads inside `tokio::select!` against a
/// watchdog tick. `recv_msg` there is cancellation-UNSAFE — a dropped
/// future loses partially-read bytes and desyncs the stream ("frame too
/// large", seen on VMs under sustained input). `read_buf` + `try_decode`
/// with a persistent buffer is cancel-safe and must deliver every frame.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn buffered_reader_survives_select_cancellation() {
    use spacing_core::event::InputEvent;
    use tokio::io::AsyncReadExt;

    let dir = tempfile::tempdir().unwrap();
    let server_id = cert::ensure_identity(&dir.path().join("srv")).unwrap();
    let client_id = cert::ensure_identity(&dir.path().join("cli")).unwrap();

    let mut client_peers = PeerStore::default();
    client_peers.insert("srv".into(), server_id.fingerprint.clone());
    let mut server_peers = PeerStore::default();
    server_peers.insert("cli".into(), client_id.fingerprint.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let srv_peers = std::sync::Arc::new(std::sync::RwLock::new(server_peers.clone()));
    let pairing_open = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let peers_path = dir.path().join("peers.toml");

    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        accept_server(tcp, server_id.clone(), srv_peers, pairing_open, peers_path)
            .await
            .unwrap()
    });

    let mut conn = connect_client(
        tcp_connect(addr).await,
        "localhost",
        client_id.clone(),
        move |fp| client_peers.allow(fp),
    )
    .await
    .unwrap();
    conn::send_msg(
        &mut conn.wr_half,
        &Message::Hello {
            version: spacing_core::protocol::PROTOCOL_VERSION,
            hostname: "cli".into(),
        },
    )
    .await
    .unwrap();
    let server_conn = server.await.unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Message>(256);
    tokio::spawn(async move {
        let mut wr = server_conn.wr;
        while let Some(m) = rx.recv().await {
            if conn::send_msg(&mut wr, &m).await.is_err() {
                break;
            }
        }
    });
    tokio::spawn(async move {
        let mut rd = server_conn.rd;
        while conn::recv_msg(&mut rd).await.is_ok() {}
    });

    const N: u32 = 5_000;
    tokio::spawn(async move {
        for i in 0..N {
            if tx.send(Message::Input(InputEvent::KeyDown(0x04))).await.is_err() {
                return;
            }
            let _ = i;
        }
    });

    // Client loop exactly like src/client.rs: read_buf + persistent
    // netbuf + try_decode inside select! with a fast cancelling tick.
    let mut rd = conn.rd_half;
    let mut netbuf: Vec<u8> = Vec::with_capacity(16 * 1024);
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(1));
    let mut got = 0u32; // counts Input frames (HelloAck also flows through)
    let mut saw_ack = false;
    while got < N {
        tokio::select! {
            r = rd.read_buf(&mut netbuf) => {
                let n = r.unwrap();
                if n == 0 { panic!("server closed early at {got}"); }
                loop {
                    match conn::try_decode(&mut netbuf) {
                        Ok(Some(Message::HelloAck { .. })) => saw_ack = true,
                        Ok(Some(Message::Input(_))) => got += 1,
                        Ok(Some(other)) => panic!("unexpected {other:?}"),
                        Ok(None) => break,
                        Err(e) => panic!("desync at {got}: {e}"),
                    }
                }
            }
            _ = tick.tick() => {}
        }
    }
    assert!(saw_ack);
    assert_eq!(got, N);
}
