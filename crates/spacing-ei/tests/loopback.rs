//! In-process ei protocol loopback: a fake portal (eis side) on one end
//! of a UnixStream pair, our ei client code on the other. Exercises the
//! real wire framing — the same bytes a GNOME/KDE portal would send and
//! receive.

use reis::ei;
use reis::eis;
use reis::enumflags2::BitFlags;
use reis::event::{DeviceCapability, EiEvent, EiEventConverter};
use reis::handshake::{ei_handshake_blocking, EisHandshaker, EisHandshakeResp};
use reis::request::{EisRequest, EisRequestConverter};
use reis::PendingRequestResult;
use spacing_core::event::{InputEvent, MouseButton};
use spacing_core::hidmap::hid_to_evdev;
use spacing_ei::{btn_code, EiDecoder, EiInterfaces};
use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::time::Duration;

fn caps() -> BitFlags<DeviceCapability> {
    DeviceCapability::Keyboard
        | DeviceCapability::Pointer
        | DeviceCapability::Button
        | DeviceCapability::Scroll
}

const EVENTS: &[InputEvent] = &[
    InputEvent::KeyDown(0x04), // 'a'
    InputEvent::KeyUp(0x04),
    InputEvent::MouseMoveRel { dx: 5, dy: -3 },
    InputEvent::ButtonDown(MouseButton::Left),
    InputEvent::ButtonUp(MouseButton::Left),
    InputEvent::Wheel { dx: 0, dy: 2 },
];

/// Server-side (fake portal) handshake — blocks until the client finishes.
fn eis_handshake(ctx: &eis::Context) -> EisHandshakeResp {
    let mut hs = EisHandshaker::new(ctx, 1);
    loop {
        ctx.read().expect("eis read");
        while let Some(res) = ctx.pending_request() {
            if let PendingRequestResult::Request(req) = res {
                if let Some(resp) = hs.handle_request(req).expect("handshake") {
                    return resp;
                }
            }
        }
    }
}

/// Wait for the client to bind, then create one device advertising the
/// bound capabilities.
fn wait_bind_make_device(ctx: &eis::Context, conv: &mut EisRequestConverter) -> reis::request::Device {
    loop {
        ctx.read().expect("eis read");
        while let Some(res) = ctx.pending_request() {
            if let PendingRequestResult::Request(req) = res {
                conv.handle_request(req).expect("request");
            }
        }
        while let Some(req) = conv.next_request() {
            if let EisRequest::Bind(bind) = req {
                return bind.seat.add_device(
                    Some("spacing-test"),
                    eis::device::DeviceType::Virtual,
                    bind.capabilities,
                    |_| {},
                );
            }
        }
    }
}

/// Pump the ei socket once: feed raw events into the converter and
/// collect the resulting high-level events.
fn drain_ei(ctx: &ei::Context, conv: &mut EiEventConverter, out: &mut Vec<EiEvent>) {
    ctx.read().expect("ei read");
    while let Some(res) = ctx.pending_event() {
        if let PendingRequestResult::Request(ev) = res {
            conv.handle_event(ev).expect("event");
        }
    }
    while let Some(ev) = conv.next_event() {
        out.push(ev);
    }
}

/// Receiver direction: eis (portal) emits → our decoder → InputEvent.
#[test]
fn eis_emits_ei_receiver_decodes() {
    let (client, server) = UnixStream::pair().unwrap();
    let srv = std::thread::spawn(move || {
        let ctx = eis::Context::new(server).unwrap();
        let resp = eis_handshake(&ctx);
        assert!(matches!(
            resp.context_type,
            eis::handshake::ContextType::Receiver
        ));
        let mut conv = EisRequestConverter::new(&ctx, resp, 1);
        let conn = conv.handle().clone();
        let _seat = conn.add_seat(Some("test-seat"), caps());
        ctx.flush().unwrap();

        let device = wait_bind_make_device(&ctx, &mut conv);
        device.resumed();
        device.start_emulating(1);
        for ev in EVENTS {
            match ev {
                InputEvent::KeyDown(h) | InputEvent::KeyUp(h) => {
                    let state = if matches!(ev, InputEvent::KeyDown(_)) {
                        eis::keyboard::KeyState::Press
                    } else {
                        eis::keyboard::KeyState::Released
                    };
                    device
                        .interface::<eis::Keyboard>()
                        .unwrap()
                        .key(u32::from(hid_to_evdev(*h).unwrap()), state);
                }
                InputEvent::MouseMoveRel { dx, dy } => device
                    .interface::<eis::Pointer>()
                    .unwrap()
                    .motion_relative(*dx as f32, *dy as f32),
                InputEvent::ButtonDown(b) | InputEvent::ButtonUp(b) => {
                    let state = if matches!(ev, InputEvent::ButtonDown(_)) {
                        eis::button::ButtonState::Press
                    } else {
                        eis::button::ButtonState::Released
                    };
                    device
                        .interface::<eis::Button>()
                        .unwrap()
                        .button(btn_code(*b), state);
                }
                InputEvent::Wheel { dx, dy } => device
                    .interface::<eis::Scroll>()
                    .unwrap()
                    .scroll_discrete(dx * 120, dy * 120),
            }
            device.frame(1);
        }
        device.stop_emulating();
        ctx.flush().unwrap();
        // keep the socket alive until the client drained the queue
        std::thread::sleep(Duration::from_millis(500));
    });

    let got = std::thread::spawn(move || {
        let ctx = ei::Context::new(client).unwrap();
        let resp =
            ei_handshake_blocking(&ctx, "loopback", ei::handshake::ContextType::Receiver).unwrap();
        let mut conv = EiEventConverter::new(&ctx, resp);
        let mut dec = EiDecoder::default();
        let mut got = Vec::new();
        while got.len() < EVENTS.len() {
            let mut batch = Vec::new();
            drain_ei(&ctx, &mut conv, &mut batch);
            for ev in &batch {
                if let EiEvent::SeatAdded(sa) = ev {
                    sa.seat.bind_capabilities(caps());
                    let _ = ctx.flush();
                }
                if let Some(ie) = dec.decode(ev) {
                    got.push(ie);
                }
            }
        }
        got
    })
    .join()
    .unwrap();
    srv.join().unwrap();
    assert_eq!(got, EVENTS);
}

/// Sender direction: our emit → eis (portal) receives raw requests.
#[test]
fn ei_sender_emits_eis_receives() {
    let (client, server) = UnixStream::pair().unwrap();
    let (tx, rx) = mpsc::channel::<Vec<String>>();

    let srv = std::thread::spawn(move || {
        let ctx = eis::Context::new(server).unwrap();
        let resp = eis_handshake(&ctx);
        assert!(matches!(
            resp.context_type,
            eis::handshake::ContextType::Sender
        ));
        let mut conv = EisRequestConverter::new(&ctx, resp, 1);
        let conn = conv.handle().clone();
        let _seat = conn.add_seat(Some("test-seat"), caps());
        ctx.flush().unwrap();

        let mut bound = false;
        let mut got = Vec::new();
        while got.len() < EVENTS.len() {
            ctx.read().unwrap();
            while let Some(res) = ctx.pending_request() {
                if let PendingRequestResult::Request(req) = res {
                    conv.handle_request(req).unwrap();
                }
            }
            while let Some(req) = conv.next_request() {
                match req {
                    EisRequest::Bind(b) if !bound => {
                        bound = true;
                        b.seat
                            .add_device(
                                Some("spacing-test"),
                                eis::device::DeviceType::Virtual,
                                b.capabilities,
                                |_| {},
                            )
                            .resumed();
                        ctx.flush().unwrap();
                    }
                    EisRequest::KeyboardKey(k) => {
                        got.push(format!("key {} {:?}", k.key, k.state))
                    }
                    EisRequest::PointerMotion(m) => {
                        got.push(format!("motion {} {}", m.dx, m.dy))
                    }
                    EisRequest::Button(b) => {
                        got.push(format!("button {} {:?}", b.button, b.state))
                    }
                    EisRequest::ScrollDiscrete(s) => {
                        got.push(format!("scroll {} {}", s.discrete_dx, s.discrete_dy))
                    }
                    _ => {}
                }
            }
        }
        tx.send(got).unwrap();
    });

    let cli = std::thread::spawn(move || {
        let ctx = ei::Context::new(client).unwrap();
        let resp =
            ei_handshake_blocking(&ctx, "loopback", ei::handshake::ContextType::Sender).unwrap();
        let mut conv = EiEventConverter::new(&ctx, resp);
        loop {
            let mut batch = Vec::new();
            drain_ei(&ctx, &mut conv, &mut batch);
            for ev in &batch {
                match ev {
                    EiEvent::SeatAdded(sa) => {
                        sa.seat.bind_capabilities(caps());
                        let _ = ctx.flush();
                    }
                    EiEvent::DeviceAdded(da) => {
                        let dev = da.device.clone();
                        let ifs = EiInterfaces {
                            keyboard: dev.interface::<ei::Keyboard>(),
                            pointer: dev.interface::<ei::Pointer>(),
                            button: dev.interface::<ei::Button>(),
                            scroll: dev.interface::<ei::Scroll>(),
                        };
                        if ifs.keyboard.is_none() {
                            continue;
                        }
                        let serial = conv.connection().serial();
                        let ed = dev.device();
                        ed.start_emulating(serial, 1);
                        for wire in EVENTS {
                            ifs.emit(wire);
                        }
                        ed.frame(serial, 42);
                        ed.stop_emulating(serial);
                        ctx.flush().unwrap();
                        return;
                    }
                    _ => {}
                }
            }
        }
    });
    cli.join().unwrap();
    let got = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    srv.join().unwrap();
    assert_eq!(
        got,
        vec![
            "key 30 Press",
            "key 30 Released",
            "motion 5 -3",
            "button 272 Press",
            "button 272 Released",
            "scroll 0 240",
        ]
    );
}
