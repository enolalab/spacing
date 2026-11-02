//! Wayland portal capture backend: XDG InputCapture → libei.
//! Requires a desktop portal implementing InputCapture (GNOME 45+/KDE 6.1+).
//! Capture starts when the pointer crosses a configured screen-edge
//! barrier — the compositor owns the session, we consume an ei socket.

use ashpd::desktop::input_capture::{
    ActivatedBarrier, Barrier, BarrierID, BarrierPosition, Capabilities, ConnectToEISOptions,
    CreateSessionOptions, EnableOptions, InputCapture, ReleaseOptions, SetPointerBarriersOptions,
};
use ashpd::desktop::Session;
use futures::StreamExt;
use spacing_ei::EiDecoder;
use std::collections::BTreeMap;
use std::io;
use std::num::NonZeroU32;
use std::os::unix::net::UnixStream;
use tokio::sync::mpsc;

use crate::CaptureEvent;

/// Portal-backed capture. `next_event` merges two sources: decoded ei
/// input events and portal Activated/Deactivated (edge) signals.
pub struct PortalCapture {
    ic: InputCapture,
    session: Session<InputCapture>,
    rx: mpsc::Receiver<CaptureEvent>,
}

/// Screen-edge name → client name, e.g. `"right" -> "laptop"`.
pub type EdgeMap = BTreeMap<String, String>;

/// (x1, y1, x2, y2) barrier segment + its edge name + portal id.
pub fn edge_barriers(
    regions: &[(i32, i32, u32, u32)],
    edges: &EdgeMap,
) -> Vec<(Barrier, String)> {
    // Global bounding box over all zones.
    let (mut min_x, mut min_y, mut max_x, mut max_y) =
        (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for &(x, y, w, h) in regions {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x + w as i32);
        max_y = max_y.max(y + h as i32);
    }
    if regions.is_empty() {
        return Vec::new();
    }
    // ids and edge names must stay in lockstep — an Activated signal
    // carries barrier_id N → edge_names[N-1] resolves the edge name.
    let mut out = Vec::with_capacity(edges.len());
    for edge in edges.keys() {
        let pos = match edge.as_str() {
            // Barrier line must sit strictly OUTSIDE the zone rect —
            // mutter rejects barriers that overlap the monitor region.
            "left" => BarrierPosition::new(min_x - 1, min_y, min_x - 1, max_y - 1),
            "right" => BarrierPosition::new(max_x, min_y, max_x, max_y - 1),
            "top" => BarrierPosition::new(min_x, min_y - 1, max_x - 1, min_y - 1),
            "bottom" => BarrierPosition::new(min_x, max_y, max_x - 1, max_y),
            other => {
                tracing::warn!(edge = %other, "unknown edge name — skipped");
                continue;
            }
        };
        let id: BarrierID = NonZeroU32::new(out.len() as u32 + 1).unwrap();
        out.push((Barrier::new(id, pos), edge.clone()));
    }
    out
}

impl PortalCapture {
    /// Portal session: InputCapture → zones → edge barriers → enable →
    /// ei socket handshake. Events stream into an mpsc channel read by
    /// `next_event`.
    pub async fn connect(edges: &EdgeMap) -> io::Result<Self> {
        tracing::debug!("portal: InputCapture proxy");
        let ic = InputCapture::new().await.map_err(portal_err)?;
        tracing::debug!("portal: create_session");
        let session = ic
            .create_session(
                None,
                CreateSessionOptions::default()
                    .set_capabilities(Capabilities::Keyboard | Capabilities::Pointer),
            )
            .await
            .map_err(portal_err)?
            .0;

        tracing::debug!("portal: zones");
        let zones = ic
            .zones(&session, Default::default())
            .await
            .map_err(portal_err)?
            .response()
            .map_err(portal_err)?;
        let regions: Vec<(i32, i32, u32, u32)> = zones
            .regions()
            .iter()
            .map(|r| (r.x_offset(), r.y_offset(), r.width(), r.height()))
            .collect();
        let barriers = edge_barriers(&regions, edges);
        let (barrier_objs, edge_names): (Vec<Barrier>, Vec<String>) =
            barriers.into_iter().unzip();
        if !barrier_objs.is_empty() {
            tracing::debug!(n = barrier_objs.len(), "portal: set_pointer_barriers");
            let resp = ic
                .set_pointer_barriers(
                    &session,
                    &barrier_objs,
                    zones.zone_set(),
                    SetPointerBarriersOptions::default(),
                )
                .await
                .map_err(portal_err)?
                .response()
                .map_err(portal_err)?;
            let failed = resp.failed_barriers();
            if !failed.is_empty() {
                tracing::warn!(?failed, "some pointer barriers were rejected");
            }
        }
        // GNOME requires the EIS connection before enable —
        // "Not connected to EIS" otherwise.
        tracing::debug!("portal: connect_to_eis");
        let fd = ic
            .connect_to_eis(&session, ConnectToEISOptions::default())
            .await
            .map_err(portal_err)?;

        tracing::debug!("portal: enable");
        ic.enable(&session, EnableOptions::default())
            .await
            .map_err(portal_err)?;
        let ctx = reis::ei::Context::new(UnixStream::from(fd))
            .map_err(|e| io::Error::new(io::ErrorKind::ConnectionRefused, e))?;

        let (tx, rx) = mpsc::channel(256);
        // The ei event stream is !Send (callback handlers) — drive it on
        // a dedicated current-thread runtime inside its own OS thread.
        // receive_* streams borrow their InputCapture proxy — created
        // inside ei_task so the borrow never crosses the spawn.
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("ei task runtime");
            tokio::task::LocalSet::new().block_on(&rt, ei_task(ctx, edge_names, tx));
        });

        Ok(Self { ic, session, rx })
    }
}

fn portal_err(e: ashpd::Error) -> io::Error {
    io::Error::other(format!("portal: {e}"))
}

/// Runs the ei socket + portal signal streams; pushes CaptureEvent.
/// Ends (channel closes) when the ei stream or session dies.
/// Owns a fresh InputCapture proxy because receive_* streams borrow it.
async fn ei_task(
    ctx: reis::ei::Context,
    edge_names: Vec<String>,
    tx: mpsc::Sender<CaptureEvent>,
) {
    use reis::event::EiEvent;
    let ic = match InputCapture::new().await {
        Ok(ic) => ic,
        Err(e) => {
            tracing::error!("portal signal proxy failed: {e}");
            return;
        }
    };
    let mut activated = match ic.receive_activated().await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("portal Activated stream failed: {e}");
            return;
        }
    };
    let mut deactivated = match ic.receive_deactivated().await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("portal Deactivated stream failed: {e}");
            return;
        }
    };
    let mut disabled = match ic.receive_disabled().await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("portal Disabled stream failed: {e}");
            return;
        }
    };
    let (_conn, mut events) = match ctx
        .handshake_tokio("spacing", reis::ei::handshake::ContextType::Receiver)
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("ei handshake failed: {e}");
            return;
        }
    };
    let mut dec = EiDecoder::default();
    loop {
        tokio::select! {
            ev = events.next() => {
                match ev {
                    Some(Ok(EiEvent::SeatAdded(sa))) => {
                        sa.seat.bind_capabilities(
                            reis::event::DeviceCapability::Keyboard
                                | reis::event::DeviceCapability::Pointer
                                | reis::event::DeviceCapability::Button
                                | reis::event::DeviceCapability::Scroll,
                        );
                        let _ = ctx.flush();
                    }
                    Some(Ok(e)) => {
                        if let Some(ie) = dec.decode(&e) {
                            if tx.send(CaptureEvent::Input(ie)).await.is_err() { return; }
                        }
                    }
                    Some(Err(e)) => {
                        tracing::error!("ei stream error: {e}");
                        let _ = tx.send(CaptureEvent::Deactivated).await;
                        return;
                    }
                    None => {
                        tracing::warn!("ei stream ended");
                        let _ = tx.send(CaptureEvent::Deactivated).await;
                        return;
                    }
                }
            }
            act = activated.next() => {
                let Some(act) = act else { return };
                let zone = match act.barrier_id() {
                    Some(ActivatedBarrier::Barrier(id)) => edge_names
                        .get(id.get() as usize - 1)
                        .cloned()
                        .unwrap_or_else(|| format!("barrier-{}", id.get())),
                    _ => "unknown".into(),
                };
                if tx.send(CaptureEvent::Activated { zone }).await.is_err() { return; }
            }
            _ = deactivated.next() => {
                if tx.send(CaptureEvent::Deactivated).await.is_err() { return; }
            }
            _ = disabled.next() => {
                let _ = tx.send(CaptureEvent::Deactivated).await;
                return;
            }
        }
    }
}

#[async_trait::async_trait]
impl crate::CaptureBackend for PortalCapture {
    /// Capture is barrier-driven — the compositor already grabbed when
    /// the edge was crossed; grab() is a no-op on the portal path.
    async fn grab(&mut self) -> io::Result<()> {
        Ok(())
    }

    /// Release the active capture — the compositor returns the pointer.
    async fn ungrab(&mut self) -> io::Result<()> {
        self.ic
            .release(&self.session, ReleaseOptions::default())
            .await
            .map_err(portal_err)
    }

    async fn next_event(&mut self) -> Option<CaptureEvent> {
        self.rx.recv().await
    }
}

impl Drop for PortalCapture {
    fn drop(&mut self) {
        // Session dies with our D-Bus connection; barrier + capture
        // state is cleaned up by the portal on disconnect.
        tracing::debug!("portal capture dropped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regions() -> Vec<(i32, i32, u32, u32)> {
        // two 1920x1080 monitors side by side
        vec![(0, 0, 1920, 1080), (1920, 0, 1920, 1080)]
    }

    #[test]
    fn right_edge_barrier_sits_outside_zone() {
        let edges: EdgeMap = [("right".to_string(), "laptop".to_string())].into_iter().collect();
        let out = edge_barriers(&regions(), &edges);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1, "right");
    }

    #[test]
    fn all_four_edges_produce_barriers() {
        let edges: EdgeMap = ["left", "right", "top", "bottom"]
            .into_iter()
            .map(|e| (e.to_string(), "x".to_string()))
            .collect();
        assert_eq!(edge_barriers(&regions(), &edges).len(), 4);
    }

    #[test]
    fn unknown_edge_is_skipped() {
        let edges: EdgeMap = [("diagonal".to_string(), "x".to_string())].into_iter().collect();
        assert!(edge_barriers(&regions(), &edges).is_empty());
    }

    #[test]
    fn empty_regions_yield_no_barriers() {
        let edges: EdgeMap = [("right".to_string(), "x".to_string())].into_iter().collect();
        assert!(edge_barriers(&[], &edges).is_empty());
    }
}
