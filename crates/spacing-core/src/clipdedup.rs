/// Latest-wins clipboard dedup with echo suppression.
///
/// Problem: we forward local text A to the peer → peer applies it →
/// peer's watcher sees "A" as a change → forwards A back → our watcher
/// sees "A" → forwards → infinite ping-pong. `applied_remote` records
/// what we wrote locally so the watcher ignores that exact echo.
#[derive(Debug, Default)]
pub struct ClipDedup {
    /// Last text observed on the local clipboard (None = empty/unknown).
    last_seen: Option<String>,
    /// Last text we applied locally on behalf of the remote.
    last_applied: Option<String>,
}

impl ClipDedup {
    /// Watcher tick. Returns `Some(text)` only for a real local change —
    /// never for an unchanged value, an empty clipboard, or an echo of
    /// `applied_remote` text.
    pub fn poll(&mut self, current: Option<&str>) -> Option<String> {
        let Some(text) = current else {
            self.last_seen = None;
            return None;
        };
        if self.last_seen.as_deref() == Some(text) {
            return None; // unchanged
        }
        self.last_seen = Some(text.into());
        if self.last_applied.as_deref() == Some(text) {
            return None; // echo of what we applied for the remote
        }
        Some(text.into())
    }

    /// Record text just applied locally from the remote peer so the next
    /// `poll` does not forward it straight back.
    pub fn applied_remote(&mut self, text: &str) {
        self.last_seen = Some(text.into());
        self.last_applied = Some(text.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_real_changes_once() {
        let mut d = ClipDedup::default();
        assert_eq!(d.poll(Some("hello")).as_deref(), Some("hello"));
        assert_eq!(d.poll(Some("hello")), None); // unchanged
        assert_eq!(d.poll(Some("world")).as_deref(), Some("world"));
    }

    #[test]
    fn empty_clipboard_is_not_forwarded() {
        let mut d = ClipDedup::default();
        assert_eq!(d.poll(None), None);
        assert_eq!(d.poll(Some("x")).as_deref(), Some("x"));
        assert_eq!(d.poll(None), None); // cleared — not propagated
        assert_eq!(d.poll(Some("x")).as_deref(), Some("x")); // re-copy is a change
    }

    #[test]
    fn remote_applied_text_is_not_echoed() {
        let mut d = ClipDedup::default();
        d.applied_remote("from-remote");
        // watcher now sees the clipboard holding the applied text
        assert_eq!(d.poll(Some("from-remote")), None);
        // …but a real local copy afterwards still forwards
        assert_eq!(d.poll(Some("local")).as_deref(), Some("local"));
    }

    #[test]
    fn user_recopying_same_text_after_remote_apply_is_suppressed() {
        // Edge case: remote applied "x", user copies "x" again — dedup
        // cannot distinguish it from the echo; documented limitation.
        let mut d = ClipDedup::default();
        d.applied_remote("x");
        assert_eq!(d.poll(Some("x")), None);
    }

    #[test]
    fn roundtrip_stops_ping_pong() {
        // Simulate both sides: A changes locally, gets forwarded, B
        // applies it, B's watcher must stay silent.
        let mut a = ClipDedup::default();
        let mut b = ClipDedup::default();
        let out_a = a.poll(Some("clip-A")).unwrap();
        b.applied_remote(&out_a);
        assert_eq!(b.poll(Some("clip-A")), None); // no echo back
        // B changes its own clipboard
        let out_b = b.poll(Some("clip-B")).unwrap();
        a.applied_remote(&out_b);
        assert_eq!(a.poll(Some("clip-B")), None);
    }
}
