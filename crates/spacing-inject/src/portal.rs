//! Wayland portal injection backend: XDG RemoteDesktop → libei.
//! The app is the ei *sender*; the compositor's EIS side injects into
//! the real session. Requires a portal implementing RemoteDesktop +
//! ConnectToEIS (GNOME 42+/KDE 5.27+); the user approves the session
//! once via the portal dialog.

use ashpd::desktop::remote_desktop::{
    ConnectToEISOptions, DeviceType, RemoteDesktop, SelectDevicesOptions, StartOptions,
};
use ashpd::desktop::{CreateSessionOptions, Session};
use futures::StreamExt;
use spacing_core::event::{InputEvent, MouseButton};
use spacing_core::hidmap::hid_to_evdev;
use spacing_ei::EiInterfaces;
use std::collections::HashSet;
use std::io;
use std::os::unix::net::UnixStream;
use std::time::Instant;
use tokio::sync::{mpsc, oneshot};

const SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(100);

enum InjectCmd {
    Event(InputEvent),
    /// Key-up everything held, stop emulating, then ack.
    ReleaseAll(oneshot::Sender<()>),
}

/// Portal-backed injector. `inject` forwards events to the ei task,
/// which owns the (non-Send) event stream and device state.
pub struct PortalInject {
    rd: RemoteDesktop,
    session: Session<RemoteDesktop>,
    tx: mpsc::Sender<InjectCmd>,
}

impl PortalInject {
    /// RemoteDesktop session: create → select keyboard+pointer →
    /// start (portal dialog on first run) → ConnectToEIS → ei socket.
    pub async fn connect() -> io::Result<Self> {
        let rd = RemoteDesktop::new().await.map_err(portal_err)?;
        let session = rd
            .create_session(CreateSessionOptions::default())
            .await
            .map_err(portal_err)?;
        rd.select_devices(
            &session,
            SelectDevicesOptions::default()
                .set_devices(DeviceType::Keyboard | DeviceType::Pointer),
        )
        .await
        .map_err(portal_err)?
        .response()
        .map_err(portal_err)?;
        rd.start(&session, None, StartOptions::default())
            .await
            .map_err(portal_err)?
            .response()
            .map_err(portal_err)?;

        let fd = rd
            .connect_to_eis(&session, ConnectToEISOptions::default())
            .await
            .map_err(portal_err)?;
        let ctx = reis::ei::Context::new(UnixStream::from(fd))
            .map_err(|e| io::Error::new(io::ErrorKind::ConnectionRefused, e))?;

        let (tx, rx) = mpsc::channel(1024);
        // Same shape as PortalCapture: the ei event stream is !Send —
        // run it on a dedicated current-thread runtime.
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("eis task runtime");
            tokio::task::LocalSet::new().block_on(&rt, eis_task(ctx, rx));
        });

        Ok(Self { rd, session, tx })
    }
}

fn portal_err(e: ashpd::Error) -> io::Error {
    io::Error::other(format!("portal: {e}"))
}

/// Capability needed to emit a given wire event.
fn needed_cap(ev: &InputEvent) -> reis::event::DeviceCapability {
    use reis::event::DeviceCapability as C;
    match ev {
        InputEvent::KeyDown(_) | InputEvent::KeyUp(_) => C::Keyboard,
        InputEvent::MouseMoveRel { .. } => C::Pointer,
        InputEvent::ButtonDown(_) | InputEvent::ButtonUp(_) => C::Button,
        InputEvent::Wheel { .. } => C::Scroll,
    }
}

/// ei sender state machine. Owns the event stream, the device list
/// (populated from DeviceAdded), emulation state and the held set so
/// `release_all` can key-up exactly what was pressed — even if the
/// compositor pauses/resumes emulation meanwhile.
async fn eis_task(ctx: reis::ei::Context, mut rx: mpsc::Receiver<InjectCmd>) {
    use reis::event::{DeviceCapability as C, EiEvent};
    let (_conn, mut events) = match ctx
        .handshake_tokio("spacing", reis::ei::handshake::ContextType::Sender)
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("ei sender handshake failed: {e}");
            return;
        }
    };

    let mut devices: Vec<reis::event::Device> = Vec::new();
    // (evdev code, is_button) currently held on the emulated device
    let mut held: HashSet<(u16, bool)> = HashSet::new();
    let mut emulating = false;
    let mut sequence = 0u32;
    let mut last_serial = 0u32;
    let t0 = Instant::now();

    loop {
        tokio::select! {
            ev = events.next() => {
                match ev {
                    Some(Ok(EiEvent::SeatAdded(sa))) => {
                        sa.seat.bind_capabilities(
                            C::Keyboard | C::Pointer | C::Button | C::Scroll,
                        );
                        let _ = ctx.flush();
                    }
                    Some(Ok(EiEvent::DeviceAdded(da))) => {
                        tracing::info!(name = ?da.device.name(), "ei device added");
                        devices.push(da.device);
                    }
                    Some(Ok(EiEvent::DeviceRemoved(dr))) => {
                        devices.retain(|d| *d != dr.device);
                        emulating = false;
                    }
                    Some(Ok(EiEvent::DeviceResumed(r))) => {
                        last_serial = r.serial;
                    }
                    Some(Ok(EiEvent::DevicePaused(_))) => {
                        // compositor paused emulation — re-start on next event
                        emulating = false;
                    }
                    Some(Ok(EiEvent::Disconnected(_))) => {
                        tracing::warn!("ei disconnected");
                        return;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        tracing::error!("ei stream error: {e}");
                        return;
                    }
                    None => {
                        tracing::warn!("ei stream ended");
                        return;
                    }
                }
            }
            cmd = rx.recv() => {
                let Some(cmd) = cmd else { return };
                match cmd {
                    InjectCmd::Event(iev) => {
                        let Some(d) =
                            devices.iter().find(|d| d.has_capability(needed_cap(&iev)))
                        else {
                            // device not announced yet — drop, not fatal
                            continue;
                        };
                        if !emulating {
                            d.device().start_emulating(last_serial, sequence);
                            sequence += 1;
                            emulating = true;
                        }
                        let ifaces = EiInterfaces {
                            keyboard: d.interface(),
                            pointer: d.interface(),
                            button: d.interface(),
                            scroll: d.interface(),
                        };
                        ifaces.emit(&iev);
                        d.device().frame(last_serial, t0.elapsed().as_millis() as u64);
                        if let Err(e) = ctx.flush() {
                            tracing::error!("ei flush failed: {e}");
                            return;
                        }
                        match iev {
                            InputEvent::KeyDown(h) => {
                                if let Some(c) = hid_to_evdev(h) { held.insert((c, false)); }
                            }
                            InputEvent::KeyUp(h) => {
                                if let Some(c) = hid_to_evdev(h) { held.remove(&(c, false)); }
                            }
                            InputEvent::ButtonDown(b) => { held.insert((btn_evdev(b), true)); }
                            InputEvent::ButtonUp(b) => { held.remove(&(btn_evdev(b), true)); }
                            _ => {}
                        }
                    }
                    InjectCmd::ReleaseAll(ack) => {
                        for d in &devices {
                            let kb = d.interface::<reis::ei::Keyboard>();
                            let btn = d.interface::<reis::ei::Button>();
                            for &(code, is_btn) in &held {
                                if is_btn {
                                    if let Some(b) = &btn {
                                        b.button(u32::from(code), reis::ei::button::ButtonState::Released);
                                    }
                                } else if let Some(k) = &kb {
                                    k.key(u32::from(code), reis::ei::keyboard::KeyState::Released);
                                }
                            }
                            if emulating {
                                d.device().frame(last_serial, t0.elapsed().as_millis() as u64);
                                d.device().stop_emulating(last_serial);
                            }
                        }
                        held.clear();
                        emulating = false;
                        let _ = ctx.flush();
                        let _ = ack.send(());
                    }
                }
            }
        }
    }
}

fn btn_evdev(b: MouseButton) -> u16 {
    spacing_ei::btn_code(b) as u16
}

#[async_trait::async_trait]
impl crate::InjectionBackend for PortalInject {
    async fn inject(&mut self, ev: &InputEvent) -> io::Result<()> {
        self.tx
            .send_timeout(InjectCmd::Event(*ev), SEND_TIMEOUT)
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, format!("ei task: {e}")))
    }

    /// Key-up all held codes on the compositor and wait for the ei task
    /// to confirm — the caller's disconnect path needs this complete
    /// before the process can exit.
    async fn release_all(&mut self) -> io::Result<()> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.tx
            .send_timeout(InjectCmd::ReleaseAll(ack_tx), SEND_TIMEOUT)
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, format!("ei task: {e}")))?;
        let _ = tokio::time::timeout(SEND_TIMEOUT, ack_rx).await;
        Ok(())
    }
}

impl Drop for PortalInject {
    fn drop(&mut self) {
        // Best-effort release + session teardown; dropping the proxy
        // disconnects and the portal cleans up emulated devices.
        let (ack_tx, _) = oneshot::channel();
        let _ = self.tx.try_send(InjectCmd::ReleaseAll(ack_tx));
        let _ = (&self.rd, &self.session); // keep fields meaningful
        tracing::debug!("portal inject dropped");
    }
}
