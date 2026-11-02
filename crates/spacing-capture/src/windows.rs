//! Windows capture backend — low-level keyboard/mouse hooks.
//!
//! `WH_KEYBOARD_LL` + `WH_MOUSE_LL` on a dedicated message-pump thread
//! (same model as Barrier/Deskflow): while local, events pass through
//! (`CallNextHookEx`) but are still mirrored to the server for hotkey
//! and screen-edge detection; while remote, hooks swallow them
//! (return 1) so the local session sees nothing.
//!
//! Edge switching mirrors the portal backend's barrier UX: the OS
//! clamps the cursor at the screen edge, consecutive outward pushes
//! (`EdgeDetector`) produce `CaptureEvent::Activated`.
//!
//! No admin/udev-style setup needed — LL hooks are user-level.

use crate::{CaptureEvent, EdgeMap};
use spacing_core::edge::EdgeDetector;
use spacing_core::event::{InputEvent as Ev, MouseButton};
use spacing_core::hidmap::evdev_to_hid;
use spacing_core::keymap_win::scan_to_evdev;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};
use tokio::sync::mpsc as tmpsc;
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// Shared state between the hook callbacks and `WinHooksCapture`.
struct Shared {
    /// true ⇒ hooks swallow events (remote control active).
    suppress: AtomicBool,
    /// Events destined for the server select loop.
    tx: tmpsc::UnboundedSender<CaptureEvent>,
    /// Screen-edge watcher — only consulted while !suppress.
    edge: Mutex<EdgeDetector>,
    /// Last reported cursor position (for motion deltas).
    last_pt: Mutex<(i32, i32)>,
    /// Wheel remainder (hi-res mice emit sub-detent deltas).
    wheel_rem: Mutex<(i32, i32)>,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

fn send(ev: CaptureEvent) {
    if let Some(s) = SHARED.get() {
        let _ = s.tx.send(ev);
    }
}

fn send_input(ev: Ev) {
    send(CaptureEvent::Input(ev));
}

fn suppressed() -> bool {
    SHARED
        .get()
        .map(|s| s.suppress.load(Ordering::Relaxed))
        .unwrap_or(false)
}

// --- keyboard hook -----------------------------------------------------

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let k = &*(lparam as *const KBDLLHOOKSTRUCT);
        // Skip injected events (e.g. our own SendInput path on a
        // same-host client) — they must not re-enter the wire.
        if k.flags & LLKHF_INJECTED == 0 {
            let ext = k.flags & LLKHF_EXTENDED != 0;
            if let Some(ev_code) = scan_to_evdev(k.scanCode as u8, ext) {
                if let Some(hid) = evdev_to_hid(ev_code) {
                    let down = matches!(wparam as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
                    send_input(if down { Ev::KeyDown(hid) } else { Ev::KeyUp(hid) });
                }
            }
        }
    }
    if suppressed() {
        return 1;
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}

// --- mouse hook --------------------------------------------------------

fn wheel_out(delta: i32) -> i32 {
    // Accumulate hi-res deltas, emit whole detents (±120 = 1 notch).
    let s = SHARED.get().unwrap();
    let mut rem = s.wheel_rem.lock().unwrap();
    rem.0 += delta;
    let detents = rem.0 / 120;
    rem.0 %= 120;
    detents
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let m = &*(lparam as *const MSLLHOOKSTRUCT);
        if m.flags & LLMHF_INJECTED == 0 {
            let (x, y) = (m.pt.x, m.pt.y);
            match wparam as u32 {
                WM_MOUSEMOVE => {
                    let mut last = SHARED.get().unwrap().last_pt.lock().unwrap();
                    let (dx, dy) = (x - last.0, y - last.1);
                    *last = (x, y);
                    drop(last);
                    if dx != 0 || dy != 0 {
                        if !suppressed() {
                            if let Some(s) = SHARED.get() {
                                if let Some(edge) =
                                    s.edge.lock().unwrap().feed(x, y, dx, dy)
                                {
                                    send(CaptureEvent::Activated { zone: edge });
                                }
                            }
                        }
                        send_input(Ev::MouseMoveRel { dx, dy });
                    }
                }
                WM_LBUTTONDOWN => send_input(Ev::ButtonDown(MouseButton::Left)),
                WM_LBUTTONUP => send_input(Ev::ButtonUp(MouseButton::Left)),
                WM_RBUTTONDOWN => send_input(Ev::ButtonDown(MouseButton::Right)),
                WM_RBUTTONUP => send_input(Ev::ButtonUp(MouseButton::Right)),
                WM_MBUTTONDOWN => send_input(Ev::ButtonDown(MouseButton::Middle)),
                WM_MBUTTONUP => send_input(Ev::ButtonUp(MouseButton::Middle)),
                WM_XBUTTONDOWN | WM_XBUTTONUP => {
                    let xbtn = (m.mouseData >> 16) & 0xFFFF;
                    let b = if xbtn == XBUTTON1 as u32 {
                        MouseButton::Side
                    } else {
                        MouseButton::Extra
                    };
                    send_input(if wparam as u32 == WM_XBUTTONDOWN {
                        Ev::ButtonDown(b)
                    } else {
                        Ev::ButtonUp(b)
                    });
                }
                WM_MOUSEWHEEL => {
                    let delta = ((m.mouseData >> 16) & 0xFFFF) as i16 as i32;
                    let dy = wheel_out(delta);
                    if dy != 0 {
                        send_input(Ev::Wheel { dx: 0, dy });
                    }
                }
                WM_MOUSEHWHEEL => {
                    let delta = ((m.mouseData >> 16) & 0xFFFF) as i16 as i32;
                    let dx = wheel_out(delta);
                    if dx != 0 {
                        send_input(Ev::Wheel { dx, dy: 0 });
                    }
                }
                _ => {}
            }
        }
    }
    if suppressed() {
        return 1;
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}

// --- backend -----------------------------------------------------------

/// Capture backend driven by Win32 low-level hooks.
/// The hook thread lives for the process lifetime.
pub struct WinHooksCapture {
    rx: tmpsc::UnboundedReceiver<CaptureEvent>,
}

impl WinHooksCapture {
    /// Install both hooks and start the pump thread. `edges` selects
    /// which screen edges arm the switch (virtual-screen rect).
    pub fn connect(edges: &EdgeMap) -> io::Result<Self> {
        let (tx, rx) = tmpsc::unbounded_channel();
        let rect = unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_XVIRTUALSCREEN)
                    + GetSystemMetrics(SM_CXVIRTUALSCREEN)
                    - 1,
                GetSystemMetrics(SM_YVIRTUALSCREEN)
                    + GetSystemMetrics(SM_CYVIRTUALSCREEN)
                    - 1,
            )
        };
        let shared = Shared {
            suppress: AtomicBool::new(false),
            tx,
            edge: Mutex::new(EdgeDetector::new(rect, edges.keys().cloned())),
            last_pt: Mutex::new((0, 0)),
            wheel_rem: Mutex::new((0, 0)),
        };
        if SHARED.set(shared).is_err() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "WinHooksCapture already initialised",
            ));
        }

        let (ready_tx, ready_rx) = mpsc::channel::<io::Result<()>>();
        std::thread::spawn(move || unsafe {
            let kb = SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(keyboard_proc),
                std::ptr::null_mut(),
                0,
            );
            if kb.is_null() {
                let _ = ready_tx.send(Err(io::Error::last_os_error()));
                return;
            }
            let ms = SetWindowsHookExW(
                WH_MOUSE_LL,
                Some(mouse_proc),
                std::ptr::null_mut(),
                0,
            );
            if ms.is_null() {
                UnhookWindowsHookEx(kb);
                let _ = ready_tx.send(Err(io::Error::last_os_error()));
                return;
            }
            let _ = ready_tx.send(Ok(()));
            let mut msg: MSG = std::mem::zeroed();
            // LL hooks are dispatched by the thread's message loop.
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) != 0 {}
        });
        // Fail fast if hook installation fails (access/session problems).
        match ready_rx.recv() {
            Ok(Ok(())) => {
                tracing::info!("Windows LL hooks installed (edge-aware capture)");
                Ok(Self { rx })
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "hook thread died before init",
            )),
        }
    }
}

#[async_trait::async_trait]
impl crate::CaptureBackend for WinHooksCapture {
    /// Local → remote: swallow everything, arm nothing (edge off).
    async fn grab(&mut self) -> io::Result<()> {
        if let Some(s) = SHARED.get() {
            s.suppress.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    /// Remote → local: pass events through again and re-arm edge watch.
    async fn ungrab(&mut self) -> io::Result<()> {
        if let Some(s) = SHARED.get() {
            s.suppress.store(false, Ordering::Relaxed);
        }
        Ok(())
    }

    async fn next_event(&mut self) -> Option<CaptureEvent> {
        self.rx.recv().await
    }
}
