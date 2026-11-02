use crate::event::InputEvent;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;
pub const CLIPBOARD_MAX: usize = 1 << 20; // 1 MiB
pub const MAX_FRAME: usize = 2 << 20; // 2 MiB hard cap per frame

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Message {
    Hello { version: u16, hostname: String },
    HelloAck { version: u16 },
    PairRequest { hostname: String },
    PairAccept,
    PairReject,
    Input(InputEvent),
    ReleaseControl,
    Heartbeat,
    ClipboardSet { text: String }, // defined for v1; unused in P1
    Error { code: u16, message: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::MouseButton;
    use proptest::prelude::*;

    fn arb_event() -> impl Strategy<Value = InputEvent> {
        prop_oneof![
            any::<u16>().prop_map(InputEvent::KeyDown),
            any::<u16>().prop_map(InputEvent::KeyUp),
            (any::<i32>(), any::<i32>()).prop_map(|(dx, dy)| InputEvent::MouseMoveRel { dx, dy }),
            prop_oneof![
                Just(MouseButton::Left),
                Just(MouseButton::Right),
                Just(MouseButton::Middle),
                Just(MouseButton::Side),
                Just(MouseButton::Extra),
            ]
            .prop_flat_map(|b| {
                prop_oneof![Just(InputEvent::ButtonDown(b)), Just(InputEvent::ButtonUp(b)),]
            }),
            (any::<i32>(), any::<i32>()).prop_map(|(dx, dy)| InputEvent::Wheel { dx, dy }),
        ]
    }

    fn arb_message() -> impl Strategy<Value = Message> {
        prop_oneof![
            (any::<u16>(), "\\PC{0,64}")
                .prop_map(|(v, h)| Message::Hello { version: v, hostname: h }),
            any::<u16>().prop_map(|v| Message::HelloAck { version: v }),
            "\\PC{0,64}".prop_map(|h| Message::PairRequest { hostname: h }),
            Just(Message::PairAccept),
            Just(Message::PairReject),
            arb_event().prop_map(Message::Input),
            Just(Message::ReleaseControl),
            Just(Message::Heartbeat),
            "\\PC{0,1024}".prop_map(|t| Message::ClipboardSet { text: t }),
            (any::<u16>(), "\\PC{0,256}").prop_map(|(c, m)| Message::Error { code: c, message: m }),
        ]
    }

    proptest! {
        #[test]
        fn postcard_roundtrip(msg in arb_message()) {
            let bytes = postcard::to_allocvec(&msg).unwrap();
            let back: Message = postcard::from_bytes(&bytes).unwrap();
            prop_assert_eq!(back, msg);
        }
    }
}
