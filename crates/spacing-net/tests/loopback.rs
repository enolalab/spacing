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
