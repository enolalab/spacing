//! macOS capture backend — `CGEventTap` session tap.
//!
//! A dedicated CFRunLoop thread owns the tap: while local, events pass
//! through but are mirrored to the server (hotkeys + edge detection);
//! while remote, the callback returns NULL and swallows them.
//! Edge switching matches the portal UX — clamped-position outward
//! pushes (`EdgeDetector`) → `CaptureEvent::Activated`.
//!
//! Permissions (TCC, granted in System Settings → Privacy & Security):
//!   "Input Monitoring" — required to create the event tap.

use crate::{CaptureEvent, EdgeMap};
use spacing_core::edge::EdgeDetector;
use spacing_core::event::{InputEvent as Ev, MouseButton};
use spacing_core::hidmap::evdev_to_hid;
use spacing_core::keymap_mac::cgkey_to_evdev;
use std::ffi::c_void;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};

// --- minimal CoreGraphics/CoreFoundation FFI (stable C ABI) -------------

type CFTypeRef = *const c_void;
type CFMachPortRef = *const c_void;
type CFRunLoopRef = *const c_void;
type CFRunLoopSourceRef = *const c_void;
type CGEventRef = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}
#[repr(C)]
struct CGSize {
    width: f64,
    height: f64,
}
#[repr(C)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

type TapCb = unsafe extern "C" fn(
    proxy: *mut c_void,
    type_: u32,
    event: CGEventRef,
    user: *mut c_void,
) -> CGEventRef;

#[link(name = "CoreGraphics", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        mask: u64,
        cb: Option<TapCb>,
        user: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(port: CFMachPortRef, enable: bool);
    fn CGEventGetIntegerValueField(ev: CGEventRef, field: u32) -> i64;
    fn CGEventGetLocation(ev: CGEventRef) -> CGPoint;
    fn CGEventGetFlags(ev: CGEventRef) -> u64;
    fn CFMachPortCreateRunLoopSource(
        alloc: CFTypeRef,
        port: CFMachPortRef,
        order: i32,
    ) -> CFRunLoopSourceRef;
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, s: CFRunLoopSourceRef, mode: CFTypeRef);
    fn CFRunLoopRun();
    fn CGMainDisplayID() -> u32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    static kCFRunLoopCommonModes: CFTypeRef;
}

// CGEventType
const T_LDOWN: u32 = 1;
const T_LUP: u32 = 2;
const T_RDOWN: u32 = 3;
const T_RUP: u32 = 4;
const T_MOVED: u32 = 5;
const T_LDRAG: u32 = 6;
const T_RDRAG: u32 = 7;
const T_KEYDOWN: u32 = 10;
const T_KEYUP: u32 = 11;
const T_FLAGS: u32 = 12;
const T_SCROLL: u32 = 22;
const T_ODOWN: u32 = 25;
const T_OUP: u32 = 26;
const T_ODRAG: u32 = 27;

// CGEventField
const F_KEYCODE: u32 = 9;
const F_SCROLL_AXIS1: u32 = 11;
const F_SCROLL_AXIS2: u32 = 12;
const F_BUTTONNUM: u32 = 61;

// Modifier flag bits (CGEventFlags)
const FL_SHIFT_L: u64 = 0x2;
const FL_SHIFT_R: u64 = 0x4;
const FL_CMD_L: u64 = 0x8;
const FL_CMD_R: u64 = 0x10;
const FL_OPT_L: u64 = 0x20;
const FL_OPT_R: u64 = 0x40;
const FL_CTL_L: u64 = 0x20000;
const FL_CTL_R: u64 = 0x80000;

/// FlagsChanged → the modifier flag bit for a CGKeyCode.
fn flag_bit(cg: u16) -> Option<u64> {
    Some(match cg {
        0x38 => FL_SHIFT_L,
        0x3C => FL_SHIFT_R,
        0x37 => FL_CMD_L,
        0x36 => FL_CMD_R,
        0x3A => FL_OPT_L,
        0x3D => FL_OPT_R,
        0x3B => FL_CTL_L,
        0x3E => FL_CTL_R,
        _ => return None,
    })
}

// --- shared state -------------------------------------------------------

struct Shared {
    suppress: AtomicBool,
    tx: mpsc::Sender<CaptureEvent>,
    edge: Mutex<EdgeDetector>,
    last_pt: Mutex<(f64, f64)>,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

fn suppressed() -> bool {
    SHARED
        .get()
        .map(|s| s.suppress.load(Ordering::Relaxed))
        .unwrap_or(false)
}

fn send(ev: CaptureEvent) {
    if let Some(s) = SHARED.get() {
        let _ = s.tx.send(ev);
    }
}

fn key_event(cg: u16, down: bool) {
    if let Some(ev) = cgkey_to_evdev(cg).and_then(evdev_to_hid) {
        send(CaptureEvent::Input(if down {
            Ev::KeyDown(ev)
        } else {
            Ev::KeyUp(ev)
        }));
    }
}

unsafe extern "C" fn tap_cb(
    _proxy: *mut c_void,
    type_: u32,
    event: CGEventRef,
    _user: *mut c_void,
) -> CGEventRef {
    let s = match SHARED.get() {
        Some(s) => s,
        None => return event,
    };
    match type_ {
        T_KEYDOWN | T_KEYUP => {
            let cg = CGEventGetIntegerValueField(event, F_KEYCODE) as u16;
            key_event(cg, type_ == T_KEYDOWN);
        }
        T_FLAGS => {
            let cg = CGEventGetIntegerValueField(event, F_KEYCODE) as u16;
            if let Some(bit) = flag_bit(cg) {
                let down = CGEventGetFlags(event) & bit != 0;
                key_event(cg, down);
            }
        }
        T_MOVED | T_LDRAG | T_RDRAG | T_ODRAG => {
            let p = CGEventGetLocation(event);
            let mut last = s.last_pt.lock().unwrap();
            let dx = (p.x - last.0) as i32;
            let dy = (p.y - last.1) as i32;
            *last = (p.x, p.y);
            drop(last);
            if dx != 0 || dy != 0 {
                if !suppressed() {
                    if let Some(edge) =
                        s.edge.lock().unwrap().feed(p.x as i32, p.y as i32, dx, dy)
                    {
                        send(CaptureEvent::Activated { zone: edge });
                    }
                }
                send(CaptureEvent::Input(Ev::MouseMoveRel { dx, dy }));
            }
        }
        T_LDOWN => send(CaptureEvent::Input(Ev::ButtonDown(MouseButton::Left))),
        T_LUP => send(CaptureEvent::Input(Ev::ButtonUp(MouseButton::Left))),
        T_RDOWN => send(CaptureEvent::Input(Ev::ButtonDown(MouseButton::Right))),
        T_RUP => send(CaptureEvent::Input(Ev::ButtonUp(MouseButton::Right))),
        T_ODOWN | T_OUP => {
            let n = CGEventGetIntegerValueField(event, F_BUTTONNUM);
            let b = match n {
                2 => MouseButton::Middle,
                3 => MouseButton::Side,
                _ => MouseButton::Extra,
            };
            send(CaptureEvent::Input(if type_ == T_ODOWN {
                Ev::ButtonDown(b)
            } else {
                Ev::ButtonUp(b)
            }));
        }
        T_SCROLL => {
            let dy = CGEventGetIntegerValueField(event, F_SCROLL_AXIS1) as i32;
            let dx = CGEventGetIntegerValueField(event, F_SCROLL_AXIS2) as i32;
            if dx != 0 || dy != 0 {
                send(CaptureEvent::Input(Ev::Wheel { dx, dy }));
            }
        }
        _ => {}
    }
    if suppressed() {
        return std::ptr::null_mut(); // swallow
    }
    event
}

fn event_mask() -> u64 {
    let bit = |t: u32| 1u64 << t;
    bit(T_LDOWN)
        | bit(T_LUP)
        | bit(T_RDOWN)
        | bit(T_RUP)
        | bit(T_MOVED)
        | bit(T_LDRAG)
        | bit(T_RDRAG)
        | bit(T_KEYDOWN)
        | bit(T_KEYUP)
        | bit(T_FLAGS)
        | bit(T_SCROLL)
        | bit(T_ODOWN)
        | bit(T_OUP)
        | bit(T_ODRAG)
}

// --- backend ------------------------------------------------------------

pub struct CgTapCapture {
    rx: mpsc::Receiver<CaptureEvent>,
}

impl CgTapCapture {
    /// Create the event tap on a run-loop thread. `edges` arms the
    /// screen-edge switch (main-display bounds).
    pub fn connect(edges: &EdgeMap) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let b = unsafe { CGDisplayBounds(CGMainDisplayID()) };
        let rect = (
            b.origin.x as i32,
            b.origin.y as i32,
            (b.origin.x + b.size.width - 1.0) as i32,
            (b.origin.y + b.size.height - 1.0) as i32,
        );
        let shared = Shared {
            suppress: AtomicBool::new(false),
            tx,
            edge: Mutex::new(EdgeDetector::new(rect, edges.keys().cloned())),
            last_pt: Mutex::new((0.0, 0.0)),
        };
        if SHARED.set(shared).is_err() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "CgTapCapture already initialised",
            ));
        }

        let (ready_tx, ready_rx) = mpsc::channel::<io::Result<()>>();
        std::thread::spawn(move || unsafe {
            let port = CGEventTapCreate(
                1, // kCGSessionEventTap
                0, // kCGHeadInsertEventTap
                0, // kCGEventTapOptionDefault — intercepts
                event_mask(),
                Some(tap_cb),
                std::ptr::null_mut(),
            );
            if port.is_null() {
                let _ = ready_tx.send(Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "CGEventTapCreate failed — grant spacing \"Input Monitoring\" \
                     in System Settings → Privacy & Security",
                )));
                return;
            }
            CGEventTapEnable(port, true);
            let src = CFMachPortCreateRunLoopSource(std::ptr::null(), port, 0);
            let rl = CFRunLoopGetCurrent();
            CFRunLoopAddSource(rl, src, kCFRunLoopCommonModes);
            let _ = ready_tx.send(Ok(()));
            CFRunLoopRun();
        });
        match ready_rx.recv() {
            Ok(Ok(())) => {
                tracing::info!("macOS CGEventTap active (edge-aware capture)");
                Ok(Self { rx })
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "event-tap thread died before init",
            )),
        }
    }
}

#[async_trait::async_trait]
impl crate::CaptureBackend for CgTapCapture {
    async fn grab(&mut self) -> io::Result<()> {
        if let Some(s) = SHARED.get() {
            s.suppress.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    async fn ungrab(&mut self) -> io::Result<()> {
        if let Some(s) = SHARED.get() {
            s.suppress.store(false, Ordering::Relaxed);
        }
        Ok(())
    }

    async fn next_event(&mut self) -> Option<CaptureEvent> {
        match tokio::task::block_in_place(|| self.rx.recv()) {
            Ok(ev) => Some(ev),
            Err(_) => None,
        }
    }
}
