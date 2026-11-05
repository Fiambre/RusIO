use rusio_plugin_api::Session;
use std::any::Any;

/// One open tab: a title plus whatever the owning plugin (or the editor) draws in it.
pub struct SessionTab {
    pub title: String,
    // Declared before `_guards` so the session is dropped before its tunnels close.
    pub session: Box<dyn Session>,
    /// Tunnels this session goes through; closed when the tab is dropped.
    _guards: Vec<Box<dyn Any + Send>>,
}

impl SessionTab {
    pub fn new(title: String, session: Box<dyn Session>) -> Self {
        Self {
            title,
            session,
            _guards: Vec::new(),
        }
    }

    pub fn with_guards(mut self, guards: Vec<Box<dyn Any + Send>>) -> Self {
        self._guards = guards;
        self
    }

    /// The session may override the title (e.g. an editor marking unsaved changes).
    pub fn display_title(&self) -> String {
        self.session.title().unwrap_or_else(|| self.title.clone())
    }
}
