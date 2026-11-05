use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender};
use parking_lot::Mutex;
use ssh2::{Session, Sftp};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use rusio_plugin_api::egui::Color32;
use rusio_plugin_api::{Host, Session as PluginSession, TextFile};

use crate::SshAuth;

#[derive(Clone)]
pub struct SftpEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: Option<u64>,
    pub perm: Option<u32>,
}

#[derive(Default)]
pub struct SftpState {
    pub connected: bool,
    pub cwd: String,
    pub entries: Vec<SftpEntry>,
    pub status: String,
    pub is_error: bool,
    /// Description of the transfer/operation currently running.
    pub busy: Option<String>,
}

/// Tunables the host exposes as plugin settings.
#[derive(Clone, Copy)]
pub struct SftpOptions {
    pub connect_timeout: Duration,
    /// 0 disables keepalive.
    pub keepalive_secs: u32,
    pub edit_max_bytes: u64,
}

pub enum SftpCmd {
    ReadFile {
        path: String,
        reply: Sender<Result<Vec<u8>, String>>,
    },
    WriteFile {
        path: String,
        data: Vec<u8>,
        reply: Sender<Result<(), String>>,
    },
    List(String),
    Download {
        remote: String,
        local: PathBuf,
    },
    Upload {
        local: PathBuf,
        remote_dir: String,
    },
    Mkdir(String),
    CreateFile(String),
    Remove {
        path: String,
        is_dir: bool,
    },
    Rename {
        from: String,
        to: String,
    },
    Close,
}

/// Dialog / edit state owned by the UI thread.
#[derive(Default)]
pub struct SftpUiState {
    pub path_edit: String,
    pub shown_cwd: String,
    pub pending_edit: Option<String>,
    pub selected: Option<String>,
    pub rename: Option<(String, String)>,
    pub mkdir: Option<String>,
    pub newfile: Option<String>,
    pub delete: Option<(String, bool, String)>,
}

/// Cheap clone that lets other tabs (the editor) talk to this SFTP worker.
#[derive(Clone)]
pub struct SftpHandle {
    cmd_tx: Sender<SftpCmd>,
    is_alive: Arc<AtomicBool>,
}

impl SftpHandle {
    pub fn is_alive(&self) -> bool {
        self.is_alive.load(Ordering::Relaxed)
    }

    pub fn read_file(&self, path: String, reply: Sender<Result<Vec<u8>, String>>) {
        let _ = self.cmd_tx.send(SftpCmd::ReadFile { path, reply });
    }

    pub fn write_file(&self, path: String, data: Vec<u8>, reply: Sender<Result<(), String>>) {
        let _ = self.cmd_tx.send(SftpCmd::WriteFile { path, data, reply });
    }
}

pub struct SftpSession {
    cmd_tx: Sender<SftpCmd>,
    pub state: Arc<Mutex<SftpState>>,
    pub is_alive: Arc<AtomicBool>,
    pub ui: SftpUiState,
}

fn es<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{}{}", dir, name)
    } else {
        format!("{}/{}", dir, name)
    }
}

fn to_remote(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

pub(crate) fn open_session(
    host: &str,
    port: u16,
    username: &str,
    auth: &SshAuth,
    opts: &SftpOptions,
) -> Result<Session, String> {
    let addr = format!("{}:{}", host, port);
    let sock = addr
        .to_socket_addrs()
        .map_err(es)?
        .next()
        .ok_or_else(|| "No se pudo resolver el host".to_string())?;
    let tcp = TcpStream::connect_timeout(&sock, opts.connect_timeout).map_err(es)?;

    let mut sess = Session::new().map_err(es)?;
    sess.set_tcp_stream(tcp);
    sess.handshake().map_err(es)?;

    let res = match auth {
        SshAuth::Password(pass) => sess.userauth_password(username, pass),
        SshAuth::KeyFile { path, passphrase } => {
            sess.userauth_pubkey_file(username, None, Path::new(path), passphrase.as_deref())
        }
        SshAuth::Agent => sess.agent().and_then(|mut agent| {
            agent.connect()?;
            agent.list_identities()?;
            for id in agent.identities()? {
                if agent.userauth(username, &id).is_ok() {
                    return Ok(());
                }
            }
            Err(ssh2::Error::from_errno(ssh2::ErrorCode::Session(-1)))
        }),
    };
    res.map_err(|e| format!("Autenticación fallida: {}", e))?;
    if !sess.authenticated() {
        return Err("Autenticación rechazada por el servidor".to_string());
    }
    if opts.keepalive_secs > 0 {
        sess.set_keepalive(true, opts.keepalive_secs);
    }
    Ok(sess)
}

fn list_dir(sftp: &Sftp, path: &str) -> Result<(String, Vec<SftpEntry>), String> {
    let real = sftp.realpath(Path::new(path)).map_err(es)?;
    let real_s = to_remote(&real);
    let mut entries: Vec<SftpEntry> = sftp
        .readdir(&real)
        .map_err(es)?
        .into_iter()
        .filter_map(|(p, st)| {
            let name = p.file_name()?.to_string_lossy().to_string();
            if name == "." || name == ".." {
                return None;
            }
            Some(SftpEntry {
                name,
                is_dir: st.is_dir(),
                size: st.size.unwrap_or(0),
                mtime: st.mtime,
                perm: st.perm,
            })
        })
        .collect();
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok((real_s, entries))
}

fn download_path(sftp: &Sftp, remote: &str, local: &Path, busy: &dyn Fn(String)) -> Result<(), String> {
    let stat = sftp.stat(Path::new(remote)).map_err(es)?;
    if stat.is_dir() {
        std::fs::create_dir_all(local).map_err(es)?;
        for (p, _) in sftp.readdir(Path::new(remote)).map_err(es)? {
            let name = match p.file_name() {
                Some(n) => n.to_string_lossy().to_string(),
                None => continue,
            };
            if name == "." || name == ".." {
                continue;
            }
            download_path(sftp, &join(remote, &name), &local.join(&name), busy)?;
        }
        return Ok(());
    }

    let total = stat.size.unwrap_or(0);
    let name = remote.rsplit('/').next().unwrap_or(remote).to_string();
    let mut src = sftp.open(Path::new(remote)).map_err(es)?;
    let mut dst = std::fs::File::create(local).map_err(es)?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    let mut last_report = 0u64;
    loop {
        let n = src.read(&mut buf).map_err(es)?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n]).map_err(es)?;
        done += n as u64;
        if done - last_report >= 256 * 1024 {
            last_report = done;
            busy(format!("Descargando {}  {}", name, progress(done, total)));
        }
    }
    Ok(())
}

fn upload_path(sftp: &Sftp, local: &Path, remote: &str, busy: &dyn Fn(String)) -> Result<(), String> {
    if local.is_dir() {
        let _ = sftp.mkdir(Path::new(remote), 0o755);
        for entry in std::fs::read_dir(local).map_err(es)? {
            let entry = entry.map_err(es)?;
            let name = entry.file_name().to_string_lossy().to_string();
            upload_path(sftp, &entry.path(), &join(remote, &name), busy)?;
        }
        return Ok(());
    }

    let total = std::fs::metadata(local).map(|m| m.len()).unwrap_or(0);
    let name = local
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut src = std::fs::File::open(local).map_err(es)?;
    let mut dst = sftp.create(Path::new(remote)).map_err(es)?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    let mut last_report = 0u64;
    loop {
        let n = src.read(&mut buf).map_err(es)?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n]).map_err(es)?;
        done += n as u64;
        if done - last_report >= 256 * 1024 {
            last_report = done;
            busy(format!("Subiendo {}  {}", name, progress(done, total)));
        }
    }
    Ok(())
}

fn read_file(sftp: &Sftp, path: &str, max_bytes: u64) -> Result<Vec<u8>, String> {
    let size = sftp.stat(Path::new(path)).map_err(es)?.size.unwrap_or(0);
    if size > max_bytes {
        return Err(format!(
            "El archivo es demasiado grande para editarlo ({}; máximo {})",
            human_size(size),
            human_size(max_bytes)
        ));
    }
    let mut f = sftp.open(Path::new(path)).map_err(es)?;
    let mut buf = Vec::with_capacity(size as usize);
    f.read_to_end(&mut buf).map_err(es)?;
    Ok(buf)
}

fn write_file(sftp: &Sftp, path: &str, data: &[u8]) -> Result<(), String> {
    let mut f = sftp.create(Path::new(path)).map_err(es)?;
    f.write_all(data).map_err(es)?;
    Ok(())
}

fn remove_path(sftp: &Sftp, path: &str, is_dir: bool) -> Result<(), String> {
    if !is_dir {
        return sftp.unlink(Path::new(path)).map_err(es);
    }
    for (p, st) in sftp.readdir(Path::new(path)).map_err(es)? {
        let name = match p.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => continue,
        };
        if name == "." || name == ".." {
            continue;
        }
        remove_path(sftp, &join(path, &name), st.is_dir())?;
    }
    sftp.rmdir(Path::new(path)).map_err(es)
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} B", bytes)
    } else {
        format!("{:.1} {}", v, UNITS[i])
    }
}

fn progress(done: u64, total: u64) -> String {
    if total > 0 {
        format!("{} / {} ({}%)", human_size(done), human_size(total), done * 100 / total)
    } else {
        human_size(done)
    }
}

impl SftpSession {
    pub fn connect(
        host: String,
        port: u16,
        username: String,
        auth: SshAuth,
        opts: SftpOptions,
        repaint: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let (cmd_tx, cmd_rx): (Sender<SftpCmd>, Receiver<SftpCmd>) = unbounded();
        let state = Arc::new(Mutex::new(SftpState {
            status: format!("Conectando a {}:{}...", host, port),
            ..Default::default()
        }));
        let is_alive = Arc::new(AtomicBool::new(true));

        let st = state.clone();
        let alive = is_alive.clone();

        thread::Builder::new().name("sftp-worker".to_string()).spawn(move || {
            let fail = |msg: String| {
                {
                    let mut s = st.lock();
                    s.connected = false;
                    s.status = msg;
                    s.is_error = true;
                    s.busy = None;
                }
                alive.store(false, Ordering::Relaxed);
                repaint();
            };

            let sess = match open_session(&host, port, &username, &auth, &opts) {
                Ok(s) => s,
                Err(e) => return fail(format!("Error de conexión: {}", e)),
            };
            let sftp = match sess.sftp() {
                Ok(s) => s,
                Err(e) => return fail(format!("No se pudo iniciar SFTP: {}", e)),
            };

            match list_dir(&sftp, ".") {
                Ok((cwd, entries)) => {
                    let mut s = st.lock();
                    s.connected = true;
                    s.cwd = cwd;
                    s.entries = entries;
                    s.status = format!("Conectado a {}", host);
                    s.is_error = false;
                }
                Err(e) => return fail(format!("No se pudo listar el directorio: {}", e)),
            }
            repaint();

            let busy = |msg: String| {
                st.lock().busy = Some(msg);
                repaint();
            };

            while alive.load(Ordering::Relaxed) {
                let cmd = match cmd_rx.recv_timeout(Duration::from_secs(5)) {
                    Ok(c) => c,
                    Err(RecvTimeoutError::Timeout) => {
                        let _ = sess.keepalive_send();
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                };

                let cwd = st.lock().cwd.clone();
                let (target, result): (String, Result<String, String>) = match cmd {
                    SftpCmd::Close => break,
                    SftpCmd::ReadFile { path, reply } => {
                        busy(format!("Abriendo {}...", path));
                        let r = read_file(&sftp, &path, opts.edit_max_bytes);
                        st.lock().busy = None;
                        let _ = reply.send(r);
                        repaint();
                        continue;
                    }
                    SftpCmd::WriteFile { path, data, reply } => {
                        busy(format!("Guardando {}...", path));
                        let r = write_file(&sftp, &path, &data);
                        st.lock().busy = None;
                        let _ = reply.send(r);
                        repaint();
                        continue;
                    }
                    SftpCmd::List(p) => (p, Ok(String::new())),
                    SftpCmd::Download { remote, local } => {
                        busy(format!("Descargando {}...", remote));
                        let r = download_path(&sftp, &remote, &local, &busy)
                            .map(|_| format!("Descargado en {}", local.display()));
                        (cwd, r)
                    }
                    SftpCmd::Upload { local, remote_dir } => {
                        let name = local
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        busy(format!("Subiendo {}...", name));
                        let r = upload_path(&sftp, &local, &join(&remote_dir, &name), &busy)
                            .map(|_| format!("Subido {}", name));
                        (remote_dir, r)
                    }
                    SftpCmd::Mkdir(path) => {
                        let r = sftp
                            .mkdir(Path::new(&path), 0o755)
                            .map(|_| "Carpeta creada".to_string())
                            .map_err(es);
                        (cwd, r)
                    }
                    SftpCmd::CreateFile(path) => {
                        let r = if sftp.stat(Path::new(&path)).is_ok() {
                            Err("Ya existe un elemento con ese nombre".to_string())
                        } else {
                            sftp.create(Path::new(&path))
                                .map(|_| "Archivo creado".to_string())
                                .map_err(es)
                        };
                        (cwd, r)
                    }
                    SftpCmd::Remove { path, is_dir } => {
                        busy(format!("Eliminando {}...", path));
                        let r = remove_path(&sftp, &path, is_dir).map(|_| "Eliminado".to_string());
                        (cwd, r)
                    }
                    SftpCmd::Rename { from, to } => {
                        let r = sftp
                            .rename(Path::new(&from), Path::new(&to), None)
                            .map(|_| "Renombrado".to_string())
                            .map_err(es);
                        (cwd, r)
                    }
                };

                let listing = list_dir(&sftp, &target);
                let result_err = result.as_ref().err().cloned();
                {
                    let mut s = st.lock();
                    s.busy = None;
                    match (&result, listing) {
                        (_, Ok((new_cwd, entries))) => {
                            s.cwd = new_cwd;
                            s.entries = entries;
                            match &result {
                                Ok(msg) if !msg.is_empty() => {
                                    s.status = msg.clone();
                                    s.is_error = false;
                                }
                                Err(e) => {
                                    s.status = format!("Error: {}", e);
                                    s.is_error = true;
                                }
                                _ => {
                                    s.status = String::new();
                                    s.is_error = false;
                                }
                            }
                        }
                        (_, Err(e)) => {
                            s.status = format!("Error: {}", result_err.unwrap_or(e));
                            s.is_error = true;
                        }
                    }
                }
                repaint();
            }

            alive.store(false, Ordering::Relaxed);
            st.lock().connected = false;
            repaint();
        })?;

        Ok(Self {
            cmd_tx,
            state,
            is_alive,
            ui: SftpUiState::default(),
        })
    }

    pub fn handle(&self) -> SftpHandle {
        SftpHandle {
            cmd_tx: self.cmd_tx.clone(),
            is_alive: self.is_alive.clone(),
        }
    }

    pub fn send(&self, cmd: SftpCmd) {
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn close(&self) {
        let _ = self.cmd_tx.send(SftpCmd::Close);
        self.is_alive.store(false, Ordering::Relaxed);
    }
}

/// A remote file the host's text editor reads and writes through this SFTP worker.
pub struct SftpFile {
    pub handle: SftpHandle,
    pub path: String,
}

impl TextFile for SftpFile {
    fn path(&self) -> String {
        self.path.clone()
    }

    fn is_alive(&self) -> bool {
        self.handle.is_alive()
    }

    fn read(&self, reply: Sender<Result<Vec<u8>, String>>) {
        self.handle.read_file(self.path.clone(), reply);
    }

    fn write(&self, data: Vec<u8>, reply: Sender<Result<(), String>>) {
        self.handle.write_file(self.path.clone(), data, reply);
    }
}

impl PluginSession for SftpSession {
    fn badge(&self) -> (&'static str, Color32) {
        ("SFTP", rusio_plugin_api::theme::Theme::default().accent_primary)
    }

    fn is_alive(&self) -> bool {
        self.is_alive.load(Ordering::Relaxed)
    }

    fn ui(&mut self, ui: &mut rusio_plugin_api::egui::Ui, host: &mut dyn Host) {
        crate::sftp_view::render_sftp(ui, self);
        if let Some(path) = self.ui.pending_edit.take() {
            host.open_text_editor(Arc::new(SftpFile {
                handle: self.handle(),
                path,
            }));
        }
    }

    fn close(&mut self) {
        SftpSession::close(self);
    }
}
