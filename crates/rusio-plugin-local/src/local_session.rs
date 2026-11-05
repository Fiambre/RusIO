use portable_pty::{CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};
use parking_lot::Mutex;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use rusio_plugin_api::term::{SharedTerminal, TerminalBackend};

use crate::LocalShellType;

pub struct LocalSession {
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    pub is_alive: Arc<AtomicBool>,
    #[allow(dead_code)]
    pub title: String,
}

impl LocalSession {
    pub fn spawn(
        shell: LocalShellType,
        working_dir: Option<String>,
        cols: u16,
        rows: u16,
        terminal: SharedTerminal,
        repaint_signal: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let pty_system = NativePtySystem::default();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let (cmd_name, args) = shell.to_command_args();
        let mut cmd = CommandBuilder::new(&cmd_name);
        for arg in &args {
            cmd.arg(arg);
        }
        if let Some(dir) = working_dir {
            cmd.cwd(dir);
        }

        let _child = pair.slave.spawn_command(cmd)?;
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        let writer = Arc::new(Mutex::new(writer));
        let reply_writer = writer.clone();

        let is_alive = Arc::new(AtomicBool::new(true));
        let is_alive_clone = is_alive.clone();
        let term_clone = terminal.clone();

        let title = shell.display_name().to_string();

        // Background reader thread
        thread::Builder::new()
            .name("local-pty-reader".to_string())
            .spawn(move || {
                let mut buf = [0u8; 4096];
                while is_alive_clone.load(Ordering::Relaxed) {
                    match reader.read(&mut buf) {
                        Ok(0) => {
                            is_alive_clone.store(false, Ordering::Relaxed);
                            break;
                        }
                        Ok(n) => {
                            let chunk = &buf[..n];
                            let reply = {
                                let mut term = term_clone.lock();
                                term.feed(chunk);
                                // ConPTY asks for the cursor position (ESC[6n) on startup
                                // and stays silent until it gets an answer.
                                if chunk.windows(4).any(|w| w == b"[6n") {
                                    let (row, col) = term.cursor_report();
                                    Some(format!("[{};{}R", row, col))
                                } else {
                                    None
                                }
                            };
                            if let Some(r) = reply {
                                let mut w = reply_writer.lock();
                                let _ = w.write_all(r.as_bytes());
                                let _ = w.flush();
                            }
                            repaint_signal();
                        }
                        Err(_) => {
                            is_alive_clone.store(false, Ordering::Relaxed);
                            break;
                        }
                    }
                }
            })?;

        Ok(Self {
            master: Arc::new(Mutex::new(pair.master)),
            writer,
            is_alive,
            title,
        })
    }

    pub fn write(&self, data: &[u8]) {
        if self.is_alive.load(Ordering::Relaxed) {
            let mut writer = self.writer.lock();
            let _ = writer.write_all(data);
            let _ = writer.flush();
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        let master = self.master.lock();
        let _ = master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
    }

    pub fn close(&self) {
        self.is_alive.store(false, Ordering::Relaxed);
    }
}

impl TerminalBackend for LocalSession {
    fn write(&self, data: &[u8]) {
        LocalSession::write(self, data)
    }

    fn resize(&self, cols: u16, rows: u16) {
        LocalSession::resize(self, cols, rows)
    }

    fn is_alive(&self) -> bool {
        self.is_alive.load(Ordering::Relaxed)
    }

    fn close(&self) {
        LocalSession::close(self)
    }
}
