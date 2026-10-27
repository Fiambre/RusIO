pub mod emulator;
pub mod local_session;
pub mod ssh_session;

pub use emulator::{SharedTerminal, TerminalEmulator};
pub use local_session::LocalSession;
pub use ssh_session::SshSession;

pub enum TerminalBackend {
    Local(LocalSession),
    Ssh(SshSession),
}

pub struct TerminalSession {
    pub backend: TerminalBackend,
    pub emulator: SharedTerminal,
    pub font_size: f32,
}

impl TerminalSession {
    pub fn new_local(
        session: LocalSession,
        emulator: SharedTerminal,
    ) -> Self {
        Self {
            backend: TerminalBackend::Local(session),
            emulator,
            font_size: 14.0,
        }
    }

    pub fn new_ssh(
        session: SshSession,
        emulator: SharedTerminal,
    ) -> Self {
        Self {
            backend: TerminalBackend::Ssh(session),
            emulator,
            font_size: 14.0,
        }
    }

    pub fn write(&self, data: &[u8]) {
        match &self.backend {
            TerminalBackend::Local(s) => s.write(data),
            TerminalBackend::Ssh(s) => s.write(data),
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        match &self.backend {
            TerminalBackend::Local(s) => s.resize(cols, rows),
            TerminalBackend::Ssh(s) => s.resize(cols, rows),
        }
    }

    pub fn is_alive(&self) -> bool {
        match &self.backend {
            TerminalBackend::Local(s) => s.is_alive.load(std::sync::atomic::Ordering::Relaxed),
            TerminalBackend::Ssh(s) => s.is_alive.load(std::sync::atomic::Ordering::Relaxed),
        }
    }

    pub fn title(&self) -> &str {
        match &self.backend {
            TerminalBackend::Local(s) => &s.title,
            TerminalBackend::Ssh(s) => &s.title,
        }
    }

    pub fn close(&self) {
        match &self.backend {
            TerminalBackend::Local(s) => s.close(),
            TerminalBackend::Ssh(s) => s.close(),
        }
    }
}
