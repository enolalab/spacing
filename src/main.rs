mod cli;
mod client;
mod server;

use clap::Parser;
use spacing_core::peers::PeerStore;
use std::path::PathBuf;

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("spacing")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();
    let cli = cli::Cli::parse();
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    match cli.cmd {
        cli::Cmd::Serve { pairing, config } => server::run(dir, config, pairing).await,
        cli::Cmd::Client { to, config } => client::run(dir, to, config).await,
        cli::Cmd::Pair { addr, name } => pair(dir, &addr, name).await,
        cli::Cmd::Fingerprint => {
            let id = spacing_net::cert::ensure_identity(&dir)?;
            println!("{}", id.fingerprint);
            Ok(())
        }
    }
}

async fn pair(dir: PathBuf, addr: &str, name: Option<String>) -> anyhow::Result<()> {
    use spacing_core::protocol::Message;
    let id = spacing_net::cert::ensure_identity(&dir)?;
    let tcp = tokio::net::TcpStream::connect(addr).await?;
    let host = addr.split(':').next().unwrap_or("server").to_string();
    // Pairing = trust-on-first-use: don't verify fp yet, show it to user.
    let mut conn = spacing_net::connect_client(tcp, &host, id, |_| true).await?;
    println!("server fingerprint: {}", conn.peer_fp);
    print!("trust this fingerprint? [y/N] ");
    use std::io::Write;
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    anyhow::ensure!(line.trim().eq_ignore_ascii_case("y"), "aborted");

    let hostname = gethostname::gethostname().to_string_lossy().into_owned();
    spacing_net::conn::send_msg(&mut conn.wr_half, &Message::PairRequest { hostname }).await?;
    match spacing_net::conn::recv_msg(&mut conn.rd_half).await? {
        Message::PairAccept => {}
        Message::PairReject => {
            anyhow::bail!("server rejected pairing (restart serve with --pairing)")
        }
        other => anyhow::bail!("unexpected reply {other:?}"),
    }

    let mut peers = PeerStore::load(&dir.join("peers.toml"))?;
    let name = name.unwrap_or_else(|| addr.to_string());
    peers.insert(name.clone(), conn.peer_fp.clone());
    peers.save(&dir.join("peers.toml"))?;
    println!("paired with {name}");
    Ok(())
}
