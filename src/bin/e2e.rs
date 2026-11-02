// E2E helper for VM testing — NOT part of the product.
//   e2e emit <delay_secs>   create uinput kbd "e2e-kbd", after delay send
//                           Ctrl+Alt+1, "hello", Ctrl+Alt+0
//   e2e listen <substr>     find /dev/input device whose name contains
//                           <substr>, print events as "<type> <code> <value>"
use evdev::uinput::VirtualDeviceBuilder;
use evdev::{AttributeSet, EventType, InputEvent, Key};
use std::io;
use std::time::Duration;

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

fn cmd_listen(needle: &str) -> io::Result<()> {
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
    loop {
        match device.fetch_events() {
            Ok(batch) => {
                for e in batch {
                    if e.event_type() == EventType::SYNCHRONIZATION {
                        continue;
                    }
                    println!("{} {} {}", e.event_type().0, e.code(), e.value());
                }
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
        Some("listen") => cmd_listen(args.get(2).map(|s| s.as_str()).unwrap_or("")),
        _ => {
            eprintln!("usage: e2e emit [delay_secs] | e2e listen <name-substring>");
            std::process::exit(2);
        }
    }
}
