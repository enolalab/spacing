use crate::combo::{Combo, ModSet};
use crate::event::InputEvent;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Local,
    Remote(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Local mode: event already reached the OS — ignore.
    Drop,
    /// Remote mode: forward to active client.
    Forward(InputEvent),
    /// Intercepted hotkey → return control locally (ungrab + ReleaseControl).
    SwitchToLocal,
    /// Intercepted hotkey → grab + forward to named client.
    SwitchToRemote(String),
}

/// Pure state machine. Owns no I/O: daemon maps Decisions to
/// grab/ungrab/network side effects.
pub struct Router {
    target: Target,
    hotkey_local: Combo,
    hotkey_switch: Vec<(Combo, String)>,
    /// Screen-edge name → client name (portal barrier switching).
    edges: std::collections::BTreeMap<String, String>,
    held: HashSet<u16>,
}

impl Router {
    pub fn new(
        hotkey_local: Combo,
        hotkey_switch: Vec<(Combo, String)>,
        edges: std::collections::BTreeMap<String, String>,
    ) -> Self {
        Self {
            target: Target::Local,
            hotkey_local,
            hotkey_switch,
            edges,
            held: HashSet::new(),
        }
    }

    pub fn target(&self) -> &Target {
        &self.target
    }

    /// Called when the active client connection drops; daemon must ungrab.
    pub fn on_disconnected(&mut self) {
        self.target = Target::Local;
        self.held.clear();
    }

    /// Portal barrier activation — `edge` is the configured edge name
    /// ("left"/"right"/"top"/"bottom"), resolved to a client name here.
    pub fn on_edge(&mut self, edge: &str) -> Decision {
        let Some(name) = self.edges.get(edge).cloned() else {
            return Decision::Drop;
        };
        self.target = Target::Remote(name.clone());
        Decision::SwitchToRemote(name)
    }

    /// Portal capture ended (ESC pressed / release / session lost).
    pub fn on_capture_end(&mut self) -> Decision {
        match self.target {
            Target::Remote(_) => {
                self.target = Target::Local;
                Decision::SwitchToLocal
            }
            Target::Local => Decision::Drop,
        }
    }

    fn mods(&self) -> ModSet {
        let mut m = ModSet::default();
        for &h in &self.held {
            if let Some(mm) = ModSet::from_hid(h) {
                if mm.ctrl {
                    m.ctrl = true;
                }
                if mm.alt {
                    m.alt = true;
                }
                if mm.shift {
                    m.shift = true;
                }
                if mm.meta {
                    m.meta = true;
                }
            }
        }
        m
    }

    fn matches(&self, c: &Combo, key: u16) -> bool {
        c.key == key && self.mods() == c.mods
    }

    pub fn on_event(&mut self, ev: InputEvent) -> Decision {
        match ev {
            InputEvent::KeyDown(hid) => {
                // hotkey check BEFORE recording/forwarding.
                // NOTE: `held` is NOT cleared on switch — modifiers remain
                // physically held, and hotkeys must keep matching (e.g.
                // Ctrl+Alt+1 → Ctrl+Alt+0 without re-pressing modifiers).
                if self.matches(&self.hotkey_local, hid) {
                    self.target = Target::Local;
                    return Decision::SwitchToLocal;
                }
                for (combo, name) in &self.hotkey_switch {
                    if self.matches(combo, hid) {
                        self.target = Target::Remote(name.clone());
                        return Decision::SwitchToRemote(name.clone());
                    }
                }
                self.held.insert(hid);
                self.dispatch(ev)
            }
            InputEvent::KeyUp(hid) => {
                self.held.remove(&hid);
                self.dispatch(ev)
            }
            _ => self.dispatch(ev),
        }
    }

    fn dispatch(&self, ev: InputEvent) -> Decision {
        match self.target {
            Target::Local => Decision::Drop,
            Target::Remote(_) => Decision::Forward(ev),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combo::parse_combo;
    use crate::event::InputEvent::*;

    fn router() -> Router {
        Router::new(
            parse_combo("Ctrl+Alt+0").unwrap(),
            vec![(parse_combo("Ctrl+Alt+1").unwrap(), "laptop".into())],
            [("right".to_string(), "laptop".to_string())]
                .into_iter()
                .collect(),
        )
    }

    const CTRL: u16 = 0xE0;
    const ALT: u16 = 0xE2;
    const K0: u16 = 0x27;
    const K1: u16 = 0x1E;
    const KA: u16 = 0x04;

    #[test]
    fn hotkey_switches_to_remote() {
        let mut r = router();
        r.on_event(KeyDown(CTRL));
        r.on_event(KeyDown(ALT));
        match r.on_event(KeyDown(K1)) {
            Decision::SwitchToRemote(name) => assert_eq!(name, "laptop"),
            d => panic!("expected SwitchToRemote, got {d:?}"),
        }
        // sau khi remote: mọi key forward, kể cả keyup của chính combo
        assert_eq!(r.on_event(KeyUp(K1)), Decision::Forward(KeyUp(K1)));
    }

    #[test]
    fn local_hotkey_returns() {
        let mut r = router();
        r.on_event(KeyDown(CTRL));
        r.on_event(KeyDown(ALT));
        r.on_event(KeyDown(K1));
        r.on_event(KeyUp(K1)); // vẫn giữ ctrl+alt
        match r.on_event(KeyDown(K0)) {
            Decision::SwitchToLocal => {}
            d => panic!("expected SwitchToLocal, got {d:?}"),
        }
    }

    #[test]
    fn local_mode_drops_events() {
        let mut r = router();
        assert_eq!(r.on_event(KeyDown(KA)), Decision::Drop);
    }

    #[test]
    fn remote_mode_forwards_everything() {
        let mut r = router();
        r.on_event(KeyDown(CTRL));
        r.on_event(KeyDown(ALT));
        r.on_event(KeyDown(K1));
        assert_eq!(r.on_event(KeyDown(KA)), Decision::Forward(KeyDown(KA)));
        assert_eq!(
            r.on_event(MouseMoveRel { dx: 3, dy: -2 }),
            Decision::Forward(MouseMoveRel { dx: 3, dy: -2 })
        );
    }

    #[test]
    fn disconnect_resets_to_local() {
        let mut r = router();
        r.on_event(KeyDown(CTRL));
        r.on_event(KeyDown(ALT));
        r.on_event(KeyDown(K1));
        assert!(matches!(r.target(), Target::Remote(_)));
        r.on_disconnected();
        assert_eq!(r.target(), &Target::Local);
        assert_eq!(r.on_event(KeyDown(KA)), Decision::Drop);
    }

    #[test]
    fn edge_switches_to_mapped_client() {
        let mut r = router();
        assert_eq!(r.on_edge("right"), Decision::SwitchToRemote("laptop".into()));
        assert!(matches!(r.target(), Target::Remote(_)));
    }

    #[test]
    fn unknown_edge_drops() {
        let mut r = router();
        assert_eq!(r.on_edge("left"), Decision::Drop);
        assert_eq!(r.target(), &Target::Local);
    }

    #[test]
    fn capture_end_returns_to_local_only_when_remote() {
        let mut r = router();
        assert_eq!(r.on_capture_end(), Decision::Drop); // local — nothing to do
        r.on_edge("right");
        assert_eq!(r.on_capture_end(), Decision::SwitchToLocal);
        assert_eq!(r.target(), &Target::Local);
    }
}
