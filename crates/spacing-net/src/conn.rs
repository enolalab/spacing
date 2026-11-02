use crate::{Rd, Wr};
use spacing_core::protocol::{Message, MAX_FRAME};
use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Optional wire dump for debugging: SPACING_WIRE_DUMP=/path appends one
/// line per frame boundary ("S/R <ns> <hex>"). Off by default — zero cost
/// when unset (no file touched).
fn dump(tag: &str, bytes: &[u8]) {
    if let Ok(p) = std::env::var("SPACING_WIRE_DUMP") {
        use std::io::Write as _;
        static M: std::sync::Mutex<Option<std::fs::File>> = std::sync::Mutex::new(None);
        let mut g = M.lock().unwrap();
        if g.is_none() {
            *g = std::fs::File::create(&p).ok();
        }
        if let Some(f) = g.as_mut() {
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            let _ = writeln!(
                f,
                "{tag} {:?} {}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                hex
            );
        }
    }
}

pub async fn send_msg(wr: &mut Wr, msg: &Message) -> io::Result<()> {
    let payload =
        postcard::to_allocvec(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let len = (payload.len() as u32).to_le_bytes();
    dump("SLEN", &len);
    dump("SPLD", &payload);
    wr.write_all(&len).await?;
    wr.write_all(&payload).await?;
    wr.flush().await
}

pub async fn recv_msg(rd: &mut Rd) -> io::Result<Message> {
    let mut lenb = [0u8; 4];
    rd.read_exact(&mut lenb).await?;
    dump("RLEN", &lenb);
    let len = u32::from_le_bytes(lenb) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame too large: len={len} bytes={lenb:02x?}"),
        ));
    }
    let mut buf = vec![0u8; len];
    rd.read_exact(&mut buf).await?;
    dump("RPLD", &buf);
    postcard::from_bytes(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Cancellation-safe frame decoder: pops one complete frame off the front
/// of `buf` if present. Pair with a persistent buffer + `AsyncReadExt::
/// read_buf` — unlike `recv_msg`, bytes already appended survive a
/// `select!` cancellation, so this never desynchronizes the stream.
///
/// Returns `Ok(None)` while a frame is incomplete; `Err` on a corrupt
/// length prefix or undecodable payload (fail-close: drop the conn).
pub fn try_decode(buf: &mut Vec<u8>) -> io::Result<Option<Message>> {
    if buf.len() < 4 {
        return Ok(None);
    }
    let len = u32::from_le_bytes(buf[..4].try_into().unwrap()) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame too large: len={len} head={:02x?}", &buf[..4]),
        ));
    }
    if buf.len() < 4 + len {
        return Ok(None);
    }
    let msg = postcard::from_bytes(&buf[4..4 + len])
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    buf.drain(..4 + len);
    Ok(Some(msg))
}
