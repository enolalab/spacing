//! Text clipboard access for both server and client loops.
//! Wayland (wl-clipboard-rs) and X11 (x11rb) go through `arboard`;
//! headless machines get a no-op so injection still works.

use std::io;

pub trait Clipboard: Send {
    /// Current clipboard text; `Ok(None)` when empty or non-text content.
    fn get_text(&mut self) -> io::Result<Option<String>>;
    fn set_text(&mut self, text: &str) -> io::Result<()>;
}

/// Real desktop clipboard via arboard.
pub struct ArboardClipboard(arboard::Clipboard);

impl Clipboard for ArboardClipboard {
    fn get_text(&mut self) -> io::Result<Option<String>> {
        match self.0.get_text() {
            Ok(t) => Ok(Some(t)),
            Err(arboard::Error::ContentNotAvailable) => Ok(None),
            Err(e) => Err(io::Error::other(e.to_string())),
        }
    }

    fn set_text(&mut self, text: &str) -> io::Result<()> {
        self.0.set_text(text).map_err(|e| io::Error::other(e.to_string()))
    }
}

/// No-session fallback — clipboard sync disabled, not an error.
pub struct NullClipboard;

impl Clipboard for NullClipboard {
    fn get_text(&mut self) -> io::Result<Option<String>> {
        Ok(None)
    }

    fn set_text(&mut self, _text: &str) -> io::Result<()> {
        Ok(())
    }
}

/// Real clipboard when a desktop session exists, else `NullClipboard`.
pub fn select_clipboard() -> Box<dyn Clipboard> {
    match arboard::Clipboard::new() {
        Ok(c) => Box::new(ArboardClipboard(c)),
        Err(e) => {
            tracing::warn!("no clipboard session ({e}) — clipboard sync disabled");
            Box::new(NullClipboard)
        }
    }
}
