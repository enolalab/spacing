use crate::{Rd, Wr};
use spacing_core::protocol::{Message, MAX_FRAME};
use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub async fn send_msg(wr: &mut Wr, msg: &Message) -> io::Result<()> {
    let payload =
        postcard::to_allocvec(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let len = (payload.len() as u32).to_le_bytes();
    wr.write_all(&len).await?;
    wr.write_all(&payload).await?;
    wr.flush().await
}

pub async fn recv_msg(rd: &mut Rd) -> io::Result<Message> {
    let mut lenb = [0u8; 4];
    rd.read_exact(&mut lenb).await?;
    let len = u32::from_le_bytes(lenb) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut buf = vec![0u8; len];
    rd.read_exact(&mut buf).await?;
    postcard::from_bytes(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
