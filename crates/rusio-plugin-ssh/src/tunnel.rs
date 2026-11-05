//! SSH port forwarding (`ssh -L`): listens on 127.0.0.1 and pipes every connection through
//! a `direct-tcpip` channel to a target as seen from the gateway.
//!
//! The pump is written against the small [`Upstream`]/[`Connector`] traits instead of ssh2
//! directly so it can be tested with real TCP sockets and an in-memory fake channel.
use rusio_plugin_api::{Params, Tunnel};
use ssh2::ErrorCode;
use std::io::{self, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::sftp::{open_session, SftpOptions};
use crate::{SshAuth, SshParams};

/// Stop buffering more data for a peer that is not draining.
const BACKLOG_LIMIT: usize = 256 * 1024;
const CHUNK: usize = 32 * 1024;
const KEEPALIVE_EVERY: Duration = Duration::from_secs(15);
/// libssh2's "would block" for non-blocking sessions.
const LIBSSH2_ERROR_EAGAIN: i32 = -37;

/// The remote end of one forwarded connection. Reads/writes never block: they return
/// `ErrorKind::WouldBlock` when there is nothing to do.
pub trait Upstream: Read + Write + Send {
    /// True once the remote side has closed its end.
    fn remote_closed(&self) -> bool;
    /// Tells the remote no more data will come from our side.
    fn finish_writing(&mut self);
}

pub trait Connector: Send {
    type Chan: Upstream;
    /// Opens a channel to the target.
    fn open(&mut self) -> Result<Self::Chan, String>;
    fn keepalive(&mut self) {}
}

/// Dropping it stops the forwarder and closes every forwarded connection.
pub struct TunnelGuard {
    stop: Arc<AtomicBool>,
}

impl Drop for TunnelGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

struct Conn<U: Upstream> {
    tcp: TcpStream,
    up: U,
    to_up: Vec<u8>,
    to_tcp: Vec<u8>,
    tcp_eof: bool,
    up_finished: bool,
}

/// Writes as much of `pending` as `w` accepts. Returns whether anything moved.
fn flush<W: Write>(w: &mut W, pending: &mut Vec<u8>) -> io::Result<bool> {
    let mut moved = false;
    while !pending.is_empty() {
        match w.write(pending) {
            Ok(0) => return Err(ErrorKind::WriteZero.into()),
            Ok(n) => {
                pending.drain(..n);
                moved = true;
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(moved)
}

impl<U: Upstream> Conn<U> {
    /// Moves what can be moved in both directions. Returns (progress, finished).
    fn pump(&mut self) -> (bool, bool) {
        match self.try_pump() {
            Ok(r) => r,
            Err(_) => (true, true),
        }
    }

    fn try_pump(&mut self) -> io::Result<(bool, bool)> {
        let mut progress = false;
        let mut buf = [0u8; CHUNK];

        progress |= flush(&mut self.up, &mut self.to_up)?;
        progress |= flush(&mut self.tcp, &mut self.to_tcp)?;

        if !self.tcp_eof && self.to_up.len() < BACKLOG_LIMIT {
            match self.tcp.read(&mut buf) {
                Ok(0) => {
                    self.tcp_eof = true;
                    progress = true;
                }
                Ok(n) => {
                    self.to_up.extend_from_slice(&buf[..n]);
                    progress = true;
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                Err(e) => return Err(e),
            }
            progress |= flush(&mut self.up, &mut self.to_up)?;
        }

        if self.to_tcp.len() < BACKLOG_LIMIT {
            match self.up.read(&mut buf) {
                // Zero bytes means the remote closed; `remote_closed` below decides.
                Ok(0) => {}
                Ok(n) => {
                    self.to_tcp.extend_from_slice(&buf[..n]);
                    progress = true;
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                Err(e) => return Err(e),
            }
            progress |= flush(&mut self.tcp, &mut self.to_tcp)?;
        }

        if self.tcp_eof && self.to_up.is_empty() && !self.up_finished {
            self.up.finish_writing();
            self.up_finished = true;
            progress = true;
        }

        // The remote is done and everything it sent has reached the client.
        let finished = self.up.remote_closed() && self.to_tcp.is_empty();
        Ok((progress, finished))
    }
}

fn run<C: Connector>(listener: TcpListener, mut connector: C, stop: Arc<AtomicBool>) {
    let mut conns: Vec<Conn<C::Chan>> = Vec::new();
    let mut last_keepalive = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        let mut progress = false;

        loop {
            match listener.accept() {
                Ok((tcp, _)) => {
                    progress = true;
                    if tcp.set_nonblocking(true).is_err() {
                        continue;
                    }
                    let _ = tcp.set_nodelay(true);
                    // If the channel cannot be opened the client socket is simply dropped.
                    if let Ok(up) = connector.open() {
                        conns.push(Conn {
                            tcp,
                            up,
                            to_up: Vec::new(),
                            to_tcp: Vec::new(),
                            tcp_eof: false,
                            up_finished: false,
                        });
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }

        conns.retain_mut(|c| {
            let (moved, finished) = c.pump();
            progress |= moved;
            !finished
        });

        if last_keepalive.elapsed() >= KEEPALIVE_EVERY {
            connector.keepalive();
            last_keepalive = Instant::now();
        }
        if !progress {
            thread::sleep(Duration::from_millis(2));
        }
    }
    // `conns` and `listener` drop here, closing every socket.
}

/// Binds a loopback port and forwards it through `connector` on a background thread.
pub fn spawn_forwarder<C: Connector + 'static>(connector: C) -> Result<(u16, TunnelGuard), String> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();

    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    thread::Builder::new()
        .name("ssh-tunnel".to_string())
        .spawn(move || run(listener, connector, thread_stop))
        .map_err(|e| e.to_string())?;
    Ok((port, TunnelGuard { stop }))
}

// ---------------------------------------------------------------- ssh2 glue

struct SshChannel(ssh2::Channel);

impl Read for SshChannel {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for SshChannel {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.0.flush() {
            Err(e) if e.kind() == ErrorKind::WouldBlock => Ok(()),
            other => other,
        }
    }
}

impl Upstream for SshChannel {
    fn remote_closed(&self) -> bool {
        self.0.eof()
    }

    fn finish_writing(&mut self) {
        let _ = self.0.send_eof();
    }
}

struct SshConnector {
    sess: ssh2::Session,
    target_host: String,
    target_port: u16,
}

impl Connector for SshConnector {
    type Chan = SshChannel;

    fn open(&mut self) -> Result<SshChannel, String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match self
                .sess
                .channel_direct_tcpip(&self.target_host, self.target_port, None)
            {
                Ok(ch) => return Ok(SshChannel(ch)),
                Err(e) if matches!(e.code(), ErrorCode::Session(LIBSSH2_ERROR_EAGAIN)) => {
                    if Instant::now() > deadline {
                        return Err("tiempo de espera al abrir el canal".to_string());
                    }
                    thread::sleep(Duration::from_millis(2));
                }
                Err(e) => return Err(e.to_string()),
            }
        }
    }

    fn keepalive(&mut self) {
        let _ = self.sess.keepalive_send();
    }
}

/// Connects to the gateway described by `params` and forwards a local port to
/// `target_host:target_port`. Blocks while connecting (call from a worker thread).
pub fn open_ssh_tunnel(
    params: &Params,
    opts: &SftpOptions,
    target_host: &str,
    target_port: u16,
) -> Result<Tunnel, String> {
    let p = SshParams::from_value(params)?;
    let auth: SshAuth = p.auth;
    let sess = open_session(&p.host, p.port, &p.username, &auth, opts)
        .map_err(|e| format!("puerta de enlace {}: {e}", p.host))?;
    // Many channels share one session on a single thread, so it must never block.
    sess.set_blocking(false);

    let (local_port, guard) = spawn_forwarder(SshConnector {
        sess,
        target_host: target_host.to_string(),
        target_port,
    })?;
    Ok(Tunnel {
        local_port,
        guard: Box::new(guard),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use std::collections::VecDeque;

    /// In-memory echo "remote": hands data back in small pieces and sometimes says
    /// WouldBlock, like a real non-blocking channel.
    struct Echo {
        buf: Arc<Mutex<VecDeque<u8>>>,
        finished: bool,
        calls: usize,
    }

    impl Read for Echo {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            self.calls += 1;
            let mut q = self.buf.lock();
            if q.is_empty() {
                return if self.finished && q.is_empty() {
                    Ok(0)
                } else {
                    Err(ErrorKind::WouldBlock.into())
                };
            }
            let n = q.len().min(out.len()).min(700);
            for slot in out.iter_mut().take(n) {
                *slot = q.pop_front().unwrap();
            }
            Ok(n)
        }
    }

    impl Write for Echo {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            if self.calls % 5 == 0 {
                return Err(ErrorKind::WouldBlock.into());
            }
            let n = data.len().min(1000);
            self.buf.lock().extend(&data[..n]);
            Ok(n)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Upstream for Echo {
        fn remote_closed(&self) -> bool {
            self.finished && self.buf.lock().is_empty()
        }

        fn finish_writing(&mut self) {
            self.finished = true;
        }
    }

    struct EchoConnector {
        fail: bool,
    }

    impl Connector for EchoConnector {
        type Chan = Echo;

        fn open(&mut self) -> Result<Echo, String> {
            if self.fail {
                return Err("refused".into());
            }
            Ok(Echo {
                buf: Arc::new(Mutex::new(VecDeque::new())),
                finished: false,
                calls: 0,
            })
        }
    }

    fn pattern(len: usize, seed: u8) -> Vec<u8> {
        (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
    }

    fn echo_through(port: u16, data: Vec<u8>) -> Vec<u8> {
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut writer = client.try_clone().unwrap();
        let sender = thread::spawn(move || {
            writer.write_all(&data).unwrap();
            writer.shutdown(std::net::Shutdown::Write).unwrap();
        });
        let mut got = Vec::new();
        client.read_to_end(&mut got).unwrap();
        sender.join().unwrap();
        got
    }

    #[test]
    fn large_payload_round_trips_and_closes_cleanly() {
        let (port, _guard) = spawn_forwarder(EchoConnector { fail: false }).unwrap();
        let data = pattern(1_500_000, 7);
        let got = echo_through(port, data.clone());
        assert_eq!(got.len(), data.len());
        assert!(got == data, "payload corrupted in transit");
    }

    #[test]
    fn concurrent_connections_stay_isolated() {
        let (port, _guard) = spawn_forwarder(EchoConnector { fail: false }).unwrap();
        let handles: Vec<_> = (0..4u8)
            .map(|i| {
                thread::spawn(move || {
                    let data = pattern(200_000 + i as usize * 1000, i * 40 + 1);
                    let got = echo_through(port, data.clone());
                    got == data
                })
            })
            .collect();
        for h in handles {
            assert!(h.join().unwrap(), "a connection received another one's data");
        }
    }

    #[test]
    fn failed_channel_closes_the_client_instead_of_hanging() {
        let (port, _guard) = spawn_forwarder(EchoConnector { fail: true }).unwrap();
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut buf = [0u8; 8];
        match client.read(&mut buf) {
            Ok(0) => {}
            Err(e) if e.kind() != ErrorKind::WouldBlock && e.kind() != ErrorKind::TimedOut => {}
            other => panic!("expected the connection to be closed, got {other:?}"),
        }
    }

    #[test]
    fn dropping_the_guard_closes_the_port() {
        let (port, guard) = spawn_forwarder(EchoConnector { fail: false }).unwrap();
        assert!(TcpStream::connect(("127.0.0.1", port)).is_ok());
        drop(guard);

        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if TcpStream::connect(("127.0.0.1", port)).is_err() {
                break;
            }
            assert!(Instant::now() < deadline, "listener still accepting after the guard dropped");
            thread::sleep(Duration::from_millis(20));
        }
    }
}
