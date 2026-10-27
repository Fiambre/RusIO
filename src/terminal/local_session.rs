use portable_pty::{CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};
use parking_lot::Mutex;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use super::emulator::SharedTerminal;
use crate::config::LocalShellType;

pub struct LocalSession {
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    pub is_alive: Arc<AtomicBool>,
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
                            term_clone.lock().feed(&buf[..n]);
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
            writer: Arc::new(Mutex::new(writer)),
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
