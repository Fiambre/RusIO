use eframe::egui;
use uuid::Uuid;

use crate::terminal::TerminalSession;
use crate::vnc::VncSession;

pub enum SessionKind {
    Terminal(TerminalSession),
    Vnc {
        session: VncSession,
        texture: Option<egui::TextureHandle>,
    },
}

pub struct SessionTab {
    pub id: Uuid,
    pub title: String,
    pub kind: SessionKind,
}

impl SessionTab {
    pub fn new_terminal(title: String, terminal: TerminalSession) -> Self {
        Self {
            id: Uuid::new_v4(),
            title,
            kind: SessionKind::Terminal(terminal),
        }
    }

    pub fn new_vnc(title: String, vnc: VncSession) -> Self {
        Self {
            id: Uuid::new_v4(),
            title,
            kind: SessionKind::Vnc {
                session: vnc,
                texture: None,
            },
        }
    }

    pub fn is_alive(&self) -> bool {
        match &self.kind {
            SessionKind::Terminal(t) => t.is_alive(),
            SessionKind::Vnc { session, .. } => {
                session.is_alive.load(std::sync::atomic::Ordering::Relaxed)
            }
        }
    }

    pub fn close(&self) {
        match &self.kind {
            SessionKind::Terminal(t) => t.close(),
            SessionKind::Vnc { session, .. } => session.close(),
        }
    }
}
