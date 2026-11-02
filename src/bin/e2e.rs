// E2E helper for VM testing — NOT part of the product.
//   e2e emit <delay_secs>   create uinput kbd "e2e-kbd", after delay send
//                           Ctrl+Alt+1, "hello", Ctrl+Alt+0
//   e2e listen <substr>     find /dev/input device whose name contains
//                           <substr>, print events as "<type> <code> <value>"
use evdev::uinput::VirtualDeviceBuilder;
use evdev::{AttributeSet, EventType, InputEvent, Key, RelativeAxisType};
use std::io::{self, Write};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn ev(t: EventType, c: u16, v: i32) -> InputEvent {
    InputEvent::new(t, c, v)
}

fn emit(dev: &mut evdev::uinput::VirtualDevice, c: u16, v: i32) -> io::Result<()> {
    dev.emit(&[ev(EventType::KEY, c, v), ev(EventType::SYNCHRONIZATION, 0, 0)])
}

fn press(dev: &mut evdev::uinput::VirtualDevice, c: u16) -> io::Result<()> {
    emit(dev, c, 1)?;
    emit(dev, c, 0)
}

// evdev codes
const LEFTCTRL: u16 = 29;
const LEFTALT: u16 = 56;
const KEY_1: u16 = 2;
const KEY_0: u16 = 11;

fn cmd_emit(delay_secs: u64) -> io::Result<()> {
    let mut keys = AttributeSet::<Key>::new();
    // Must look like a keyboard to EvdevCapture::enumerate(): it requires
    // all of A..L (30..=38) in supported_keys.
    for c in 30u16..=38 {
        keys.insert(Key::new(c));
    }
    for c in [LEFTCTRL, LEFTALT, KEY_1, KEY_0, 35, 18, 38, 24] {
        keys.insert(Key::new(c));
    }
    let mut dev = VirtualDeviceBuilder::new()?
        .name("e2e-kbd")
        .with_keys(&keys)?
        .build()?;
    eprintln!("e2e-kbd created; emitting in {delay_secs}s");
    std::thread::sleep(Duration::from_secs(delay_secs));

    // Ctrl+Alt+1 -> server switches to remote "spacing-cli"
    emit(&mut dev, LEFTCTRL, 1)?;
    emit(&mut dev, LEFTALT, 1)?;
    press(&mut dev, KEY_1)?;
    std::thread::sleep(Duration::from_millis(300));

    // type "hello" — should arrive on the client VM
    for c in [35u16, 18, 38, 38, 24] {
        press(&mut dev, c)?;
        std::thread::sleep(Duration::from_millis(60));
    }
    std::thread::sleep(Duration::from_millis(300));

    // Ctrl+Alt+0 -> back to local (client gets ReleaseControl)
    press(&mut dev, KEY_0)?;
    emit(&mut dev, LEFTALT, 0)?;
    emit(&mut dev, LEFTCTRL, 0)?;
    std::thread::sleep(Duration::from_millis(300));
    eprintln!("emit done");
    Ok(())
}

/// Like `emit` but stays remote for 30s (window to kill the client),
/// then sends KEY_Q probes — a second `e2e listen e2e-kbd` reader proves
/// ungrab happened (grabbed devices deliver events ONLY to the grabber).
fn cmd_emit_hold(delay_secs: u64) -> io::Result<()> {
    let mut keys = AttributeSet::<Key>::new();
    for c in 30u16..=38 {
        keys.insert(Key::new(c));
    }
    for c in [LEFTCTRL, LEFTALT, KEY_1, 16] {
        keys.insert(Key::new(c)); // 16 = KEY_Q
    }
    let mut dev = VirtualDeviceBuilder::new()?
        .name("e2e-kbd")
        .with_keys(&keys)?
        .build()?;
    eprintln!("e2e-kbd created; emit-hold in {delay_secs}s");
    std::thread::sleep(Duration::from_secs(delay_secs));

    emit(&mut dev, LEFTCTRL, 1)?;
    emit(&mut dev, LEFTALT, 1)?;
    press(&mut dev, KEY_1)?; // -> remote
    std::thread::sleep(Duration::from_secs(30)); // remote window

    for _ in 0..5 {
        press(&mut dev, 16)?; // KEY_Q probes — visible iff ungrabbed
        std::thread::sleep(Duration::from_millis(1500));
    }
    eprintln!("emit-hold done");
    Ok(())
}

fn now_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

/// Benchmark emitter: Ctrl+Alt+1 then `count` events spaced `interval_ms`.
/// Prints "E <t0_ns> <seq>" per event — pair with `listen --ts` output
/// ("R <t1_ns> ...") to compute per-event e2e latency (same clock only).
/// mode: "key" = KEY_H presses; "mouse" = REL_X +5.
fn cmd_emit_bench(delay_secs: u64, count: u32, interval_ms: u64, mouse: bool) -> io::Result<()> {
    let mut keys = AttributeSet::<Key>::new();
    for c in 30u16..=38 {
        keys.insert(Key::new(c));
    }
    for c in [LEFTCTRL, LEFTALT, KEY_1, 35] {
        keys.insert(Key::new(c)); // 35 = KEY_H
    }
    let mut axes = AttributeSet::<RelativeAxisType>::new();
    axes.insert(RelativeAxisType::REL_X);
    axes.insert(RelativeAxisType::REL_Y);
    let mut dev = VirtualDeviceBuilder::new()?
        .name("e2e-kbd")
        .with_keys(&keys)?
        .with_relative_axes(&axes)?
        .build()?;
    eprintln!("e2e-kbd bench up; firing in {delay_secs}s");
    std::thread::sleep(Duration::from_secs(delay_secs));

    emit(&mut dev, LEFTCTRL, 1)?;
    emit(&mut dev, LEFTALT, 1)?;
    press(&mut dev, KEY_1)?; // -> remote
    std::thread::sleep(Duration::from_millis(500));

    let mut out = io::stdout().lock();
    for i in 0..count {
        let t0 = now_ns();
        if mouse {
            dev.emit(&[ev(EventType::RELATIVE, 0, 5), ev(EventType::SYNCHRONIZATION, 0, 0)])?;
        } else {
            press(&mut dev, 35)?;
        }
        writeln!(out, "E {t0} {i}")?;
        out.flush()?;
        std::thread::sleep(Duration::from_millis(interval_ms));
    }
    // keep device alive briefly so trailing events are delivered
    std::thread::sleep(Duration::from_secs(2));
    eprintln!("emit-bench done");
    Ok(())
}

/// Virtual pointer (REL_X/Y + BTN_LEFT) — usable as a real pointing
/// device for moving the cursor and clicking, incl. inside portal
/// dialogs, without any display-server-specific tooling.
fn uinput_ptr() -> io::Result<evdev::uinput::VirtualDevice> {
    let mut axes = AttributeSet::<RelativeAxisType>::new();
    axes.insert(RelativeAxisType::REL_X);
    axes.insert(RelativeAxisType::REL_Y);
    let mut keys = AttributeSet::<Key>::new();
    keys.insert(Key::new(0x110)); // BTN_LEFT
    VirtualDeviceBuilder::new()?
        .name("e2e-ptr")
        .with_relative_axes(&axes)?
        .with_keys(&keys)?
        .build()
}

/// Virtual keyboard that can emit ANY key code 1..=127.
fn uinput_kbd() -> io::Result<evdev::uinput::VirtualDevice> {
    let mut keys = AttributeSet::<Key>::new();
    for c in 1u16..=127 {
        keys.insert(Key::new(c));
    }
    VirtualDeviceBuilder::new()?
        .name("e2e-kbd2")
        .with_keys(&keys)?
        .build()
}

fn relmove(
    dev: &mut evdev::uinput::VirtualDevice,
    dx: i32,
    dy: i32,
) -> io::Result<()> {
    let mut evs = Vec::new();
    if dx != 0 {
        evs.push(ev(EventType::RELATIVE, 0, dx));
    }
    if dy != 0 {
        evs.push(ev(EventType::RELATIVE, 1, dy));
    }
    evs.push(ev(EventType::SYNCHRONIZATION, 0, 0));
    dev.emit(&evs)
}

/// `e2e pmove dx dy [reps] [ms]` — move the real cursor by (dx,dy),
/// `reps` times, one chunk per `ms` (default 1 rep, 20 ms).
fn cmd_pmove(dx: i32, dy: i32, reps: u32, ms: u64) -> io::Result<()> {
    let mut dev = uinput_ptr()?;
    std::thread::sleep(Duration::from_millis(300)); // let libinput adopt it
    for _ in 0..reps.max(1) {
        relmove(&mut dev, dx, dy)?;
        std::thread::sleep(Duration::from_millis(ms));
    }
    eprintln!("pmove done ({dx},{dy}) x{reps}");
    Ok(())
}

/// `e2e pclick` — left click at the current cursor position.
fn cmd_pclick() -> io::Result<()> {
    let mut dev = uinput_ptr()?;
    std::thread::sleep(Duration::from_millis(300));
    emit(&mut dev, 0x110, 1)?;
    emit(&mut dev, 0x110, 0)?;
    eprintln!("pclick done");
    Ok(())
}

/// `e2e pkeys c1,c2,...` — press+release each evdev key code in order.
fn cmd_pkeys(codes: &[u16]) -> io::Result<()> {
    let mut dev = uinput_kbd()?;
    std::thread::sleep(Duration::from_millis(300));
    for &c in codes {
        press(&mut dev, c)?;
        std::thread::sleep(Duration::from_millis(50));
    }
    eprintln!("pkeys done");
    Ok(())
}

/// `e2e pseq [delay]` — portal smoke sequence, all on persistent
/// devices (they must outlive the InputCapture grab):
///   push cursor far right (edge barrier) → type "hi" → Ctrl+Alt+0.
fn cmd_pseq(delay_secs: u64) -> io::Result<()> {
    let mut ptr = uinput_ptr()?;
    let mut kbd = uinput_kbd()?;
    eprintln!("e2e-ptr+e2e-kbd2 up; sequence in {delay_secs}s");
    std::thread::sleep(Duration::from_secs(delay_secs));

    eprintln!("phase: push right edge");
    for _ in 0..80 {
        relmove(&mut ptr, 1200, 0)?;
        std::thread::sleep(Duration::from_millis(15));
    }
    std::thread::sleep(Duration::from_millis(1200));

    eprintln!("phase: type hi");
    press(&mut kbd, 35)?; // H
    std::thread::sleep(Duration::from_millis(150));
    press(&mut kbd, 23)?; // I
    std::thread::sleep(Duration::from_millis(1200));

    eprintln!("phase: Ctrl+Alt+0 back to local");
    emit(&mut kbd, LEFTCTRL, 1)?;
    emit(&mut kbd, LEFTALT, 1)?;
    press(&mut kbd, KEY_0)?;
    emit(&mut kbd, LEFTALT, 0)?;
    emit(&mut kbd, LEFTCTRL, 0)?;
    std::thread::sleep(Duration::from_millis(1200));

    eprintln!("pseq done");
    Ok(())
}

fn cmd_listen(needle: &str, with_ts: bool) -> io::Result<()> {
    let mut found = None;
    for (path, device) in evdev::enumerate() {
        if device.name().unwrap_or("").contains(needle) {
            found = Some((path, device));
            break;
        }
    }
    let (path, mut device) = match found {
        Some(t) => t,
        None => {
            eprintln!("no device matching {needle:?}");
            std::process::exit(2);
        }
    };
    eprintln!("listening on {path:?} ({:?})", device.name().unwrap_or(""));
    let mut out = io::stdout().lock();
    loop {
        match device.fetch_events() {
            Ok(batch) => {
                for e in batch {
                    if e.event_type() == EventType::SYNCHRONIZATION {
                        continue;
                    }
                    if with_ts {
                        writeln!(out, "R {} {} {} {}", now_ns(), e.event_type().0, e.code(), e.value())?;
                    } else {
                        writeln!(out, "{} {} {}", e.event_type().0, e.code(), e.value())?;
                    }
                }
                out.flush()?;
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => return Err(e),
        }
    }
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("emit") => {
            let delay: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(6);
            cmd_emit(delay)
        }
        Some("emit-hold") => {
            let delay: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
            cmd_emit_hold(delay)
        }
        Some("emit-bench") => {
            let delay: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
            let count: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(200);
            let interval: u64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(10);
            let mouse = args.get(5).map(|s| s == "mouse").unwrap_or(false);
            cmd_emit_bench(delay, count, interval, mouse)
        }
        Some("pmove") => {
            let dx: i32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            let dy: i32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
            let reps: u32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(1);
            let ms: u64 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(20);
            cmd_pmove(dx, dy, reps, ms)
        }
        Some("pclick") => cmd_pclick(),
        Some("pkeys") => {
            let codes: Vec<u16> = args
                .get(2)
                .map(|s| s.split(',').filter_map(|t| t.parse().ok()).collect())
                .unwrap_or_default();
            cmd_pkeys(&codes)
        }
        Some("pseq") => {
            let delay: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
            cmd_pseq(delay)
        }
        Some("listen") => {
            let positional: Vec<&str> = args
                .iter()
                .skip(2)
                .filter(|a| !a.starts_with('-'))
                .map(|s| s.as_str())
                .collect();
            cmd_listen(
                positional.first().copied().unwrap_or(""),
                args.iter().any(|a| a == "--ts"),
            )
        }
        _ => {
            eprintln!(
                "usage: e2e emit [delay_secs] | emit-hold [delay_secs] | \
                 emit-bench [delay_s count interval_ms key|mouse] | \
                 listen <name-substring> [--ts] | pmove dx dy [reps ms] | \
                 pclick | pkeys c1,c2,.. | pseq [delay_secs]"
            );
            std::process::exit(2);
        }
    }
}
