use crossbeam_channel::{unbounded, Receiver, Sender};
use ssh2::Session;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use rusio_plugin_api::term::{SharedTerminal, TerminalBackend};

use crate::SshAuth;

pub enum SshAction {
    Write(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Close,
}

pub struct SshSession {
    action_tx: Sender<SshAction>,
    pub is_alive: Arc<AtomicBool>,
    #[allow(dead_code)]
    pub title: String,
}

impl SshSession {
    pub fn connect(
        host: String,
        port: u16,
        username: String,
        auth: SshAuth,
        cols: u16,
        rows: u16,
        connect_timeout: Duration,
        terminal: SharedTerminal,
        repaint_signal: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let (action_tx, action_rx): (Sender<SshAction>, Receiver<SshAction>) = unbounded();
        let is_alive = Arc::new(AtomicBool::new(true));
        let is_alive_clone = is_alive.clone();
        let title = format!("{}@{}:{}", username, host, port);

        let host_clone = host.clone();
        let user_clone = username.clone();

        thread::Builder::new().name("ssh-worker".to_string()).spawn(move || {
            let addr = format!("{}:{}", host_clone, port);
            let connect_msg = format!("\x1b[36m>>> Connecting to {}...\x1b[0m\r\n", addr);
            terminal.lock().feed(connect_msg.as_bytes());
            repaint_signal();

            let tcp = match TcpStream::connect_timeout(
                &addr.parse().unwrap_or_else(|_| {
                    // resolve hostname
                    use std::net::ToSocketAddrs;
                    addr.to_socket_addrs()
                        .ok()
                        .and_then(|mut iter| iter.next())
                        .unwrap_or_else(|| "127.0.0.1:22".parse().unwrap())
                }),
                connect_timeout,
            ) {
                Ok(stream) => stream,
                Err(e) => {
                    let err_msg = format!("\x1b[31;1m>>> Connection error: {}\x1b[0m\r\n", e);
                    terminal.lock().feed(err_msg.as_bytes());
                    repaint_signal();
                    is_alive_clone.store(false, Ordering::Relaxed);
                    return;
                }
            };

            let mut sess = match Session::new() {
                Ok(s) => s,
                Err(e) => {
                    let err_msg = format!("\x1b[31;1m>>> SSH session error: {}\x1b[0m\r\n", e);
                    terminal.lock().feed(err_msg.as_bytes());
                    repaint_signal();
                    is_alive_clone.store(false, Ordering::Relaxed);
                    return;
                }
            };

            sess.set_tcp_stream(tcp);
            if let Err(e) = sess.handshake() {
                let err_msg = format!("\x1b[31;1m>>> SSH handshake error: {}\x1b[0m\r\n", e);
                terminal.lock().feed(err_msg.as_bytes());
                repaint_signal();
                is_alive_clone.store(false, Ordering::Relaxed);
                return;
            }

            // Authenticate
            let auth_result = match &auth {
                SshAuth::Password(pass) => sess.userauth_password(&user_clone, pass),
                SshAuth::KeyFile { path, passphrase } => {
                    sess.userauth_pubkey_file(&user_clone, None, Path::new(path), passphrase.as_deref())
                }
                SshAuth::Agent => sess.agent().and_then(|mut agent| {
                    agent.connect()?;
                    agent.list_identities()?;
                    for id in agent.identities()? {
                        if agent.userauth(&user_clone, &id).is_ok() {
                            return Ok(());
                        }
                    }
                    Err(ssh2::Error::from_errno(ssh2::ErrorCode::Session(-1)))
                }),
            };

            if let Err(e) = auth_result {
                let err_msg = format!("\x1b[31;1m>>> SSH authentication failed: {}\x1b[0m\r\n", e);
                terminal.lock().feed(err_msg.as_bytes());
                repaint_signal();
                is_alive_clone.store(false, Ordering::Relaxed);
                return;
            }

            if !sess.authenticated() {
                let err_msg = "\x1b[31;1m>>> Authentication rejected by server.\x1b[0m\r\n";
                terminal.lock().feed(err_msg.as_bytes());
                repaint_signal();
                is_alive_clone.store(false, Ordering::Relaxed);
                return;
            }

            let mut channel = match sess.channel_session() {
                Ok(c) => c,
                Err(e) => {
                    let err_msg = format!("\x1b[31;1m>>> Failed to open channel: {}\x1b[0m\r\n", e);
                    terminal.lock().feed(err_msg.as_bytes());
                    repaint_signal();
                    is_alive_clone.store(false, Ordering::Relaxed);
                    return;
                }
            };

            if let Err(e) = channel.request_pty("xterm-256color", None, Some((cols as u32, rows as u32, 0, 0))) {
                let err_msg = format!("\x1b[31;1m>>> PTY request error: {}\x1b[0m\r\n", e);
                terminal.lock().feed(err_msg.as_bytes());
                repaint_signal();
                is_alive_clone.store(false, Ordering::Relaxed);
                return;
            }

            if let Err(e) = channel.shell() {
                let err_msg = format!("\x1b[31;1m>>> Shell start error: {}\x1b[0m\r\n", e);
                terminal.lock().feed(err_msg.as_bytes());
                repaint_signal();
                is_alive_clone.store(false, Ordering::Relaxed);
                return;
            }

            let ready_msg = format!("\x1b[32;1m>>> Connected to {}!\x1b[0m\r\n", addr);
            terminal.lock().feed(ready_msg.as_bytes());
            repaint_signal();

            // Set non-blocking on session for responsive read/write loop
            sess.set_blocking(false);

            let mut read_buf = [0u8; 8192];

            while is_alive_clone.load(Ordering::Relaxed) {
                // Check outgoing actions
                while let Ok(action) = action_rx.try_recv() {
                    match action {
                        SshAction::Write(data) => {
                            let _ = channel.write_all(&data);
                            let _ = channel.flush();
                        }
                        SshAction::Resize { cols, rows } => {
                            let _ = channel.request_pty_size(cols as u32, rows as u32, None, None);
                        }
                        SshAction::Close => {
                            let _ = channel.close();
                            is_alive_clone.store(false, Ordering::Relaxed);
                            break;
                        }
                    }
                }

                if !is_alive_clone.load(Ordering::Relaxed) {
                    break;
                }

                // Read incoming data
                match channel.read(&mut read_buf) {
                    Ok(0) => {
                        // Channel closed
                        if channel.eof() {
                            let disc_msg = "\r\n\x1b[33m>>> SSH Session closed by remote host.\x1b[0m\r\n";
                            terminal.lock().feed(disc_msg.as_bytes());
                            repaint_signal();
                            is_alive_clone.store(false, Ordering::Relaxed);
                            break;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Ok(n) => {
                        terminal.lock().feed(&read_buf[..n]);
                        repaint_signal();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => {
                        thread::sleep(Duration::from_millis(10));
                    }
                }
            }

            let _ = channel.close();
        })?;

        Ok(Self {
            action_tx,
            is_alive,
            title,
        })
    }

    pub fn write(&self, data: &[u8]) {
        if self.is_alive.load(Ordering::Relaxed) {
            let _ = self.action_tx.send(SshAction::Write(data.to_vec()));
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        let _ = self.action_tx.send(SshAction::Resize { cols, rows });
    }

    pub fn close(&self) {
        let _ = self.action_tx.send(SshAction::Close);
    }
}

impl TerminalBackend for SshSession {
    fn write(&self, data: &[u8]) {
        SshSession::write(self, data)
    }

    fn resize(&self, cols: u16, rows: u16) {
        SshSession::resize(self, cols, rows)
    }

    fn is_alive(&self) -> bool {
        self.is_alive.load(Ordering::Relaxed)
    }

    fn close(&self) {
        SshSession::close(self)
    }
}
