use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "spacing", about = "Share keyboard/mouse across LAN")]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Run as input owner (captures this machine's devices).
    Serve {
        /// Open a one-shot pairing window for one unknown client cert.
        #[arg(long)]
        pairing: bool,
        /// Config path (default ~/.config/spacing/config.toml).
        #[arg(long)]
        config: Option<std::path::PathBuf>,
    },
    /// Run as controlled machine (injects received input).
    Client {
        /// server addr host:port (overrides config connect_to)
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        config: Option<std::path::PathBuf>,
    },
    /// Pair with a server: saves its fingerprint to peers.toml.
    Pair {
        /// server addr host:port
        addr: String,
        /// friendly name to store
        #[arg(long)]
        name: Option<String>,
    },
    /// Print this machine's cert fingerprint.
    Fingerprint,
    /// Dump captured events to stdout (debug/VM testing).
    #[command(hide = true)]
    CaptureDump {
        /// edge names to arm for edge switching (e.g. --edge right)
        #[arg(long)]
        edge: Vec<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    #[test]
    fn cli_debug_assert() {
        Cli::command().debug_assert();
    }
}
