use crossbeam_channel::{unbounded, Receiver, Sender};
use parking_lot::Mutex;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use vnc::client::{AuthChoice, AuthMethod, Client, Event};
use vnc::Rect;

pub struct VncFramebuffer {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>, // RGBA 4 bytes per pixel
    pub dirty: bool,
}

impl VncFramebuffer {
    pub fn new(width: usize, height: usize) -> Self {
        let size = width * height * 4;
        let mut pixels = vec![255; size];
        // Default dark slate background
        for i in 0..width * height {
            pixels[i * 4] = 20;
            pixels[i * 4 + 1] = 24;
            pixels[i * 4 + 2] = 34;
            pixels[i * 4 + 3] = 255;
        }
        Self {
            width,
            height,
            pixels,
            dirty: true,
        }
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        if width == self.width && height == self.height {
            return;
        }
        self.width = width;
        self.height = height;
        let size = width * height * 4;
        self.pixels = vec![255; size];
        for i in 0..width * height {
            self.pixels[i * 4] = 20;
            self.pixels[i * 4 + 1] = 24;
            self.pixels[i * 4 + 2] = 34;
            self.pixels[i * 4 + 3] = 255;
        }
        self.dirty = true;
    }

    pub fn put_pixels(&mut self, rect: Rect, raw: &[u8]) {
        let rw = rect.width as usize;
        let rh = rect.height as usize;
        let rx = rect.left as usize;
        let ry = rect.top as usize;

        // VNC format is typically 32-bit (4 bytes per pixel: B, G, R, X or R, G, B, X)
        // Ensure bounds
        if rx + rw > self.width || ry + rh > self.height {
            return;
        }

        let mut src_idx = 0;
        for y in 0..rh {
            let row_y = ry + y;
            for x in 0..rw {
                let col_x = rx + x;
                let dst_idx = (row_y * self.width + col_x) * 4;
                if src_idx + 4 <= raw.len() && dst_idx + 4 <= self.pixels.len() {
                    // Standard RFB 32bpp is often B, G, R, Pad
                    let b = raw[src_idx];
                    let g = raw[src_idx + 1];
                    let r = raw[src_idx + 2];
                    self.pixels[dst_idx] = r;
                    self.pixels[dst_idx + 1] = g;
                    self.pixels[dst_idx + 2] = b;
                    self.pixels[dst_idx + 3] = 255;
                }
                src_idx += 4;
            }
        }
        self.dirty = true;
    }
}

pub enum VncAction {
    Pointer { buttons: u8, x: u16, y: u16 },
    Key { down: bool, key: u32 },
    Close,
}

pub struct VncSession {
    pub framebuffer: Arc<Mutex<VncFramebuffer>>,
    pub action_tx: Sender<VncAction>,
    pub is_alive: Arc<AtomicBool>,
    pub status_text: Arc<Mutex<String>>,
    #[allow(dead_code)]
    pub title: String,
}

impl VncSession {
    pub fn connect(
        host: String,
        port: u16,
        password: Option<String>,
        repaint_signal: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let (action_tx, action_rx): (Sender<VncAction>, Receiver<VncAction>) = unbounded();
        let is_alive = Arc::new(AtomicBool::new(true));
        let is_alive_clone = is_alive.clone();
        let title = format!("VNC {}:{}", host, port);
        let status_text = Arc::new(Mutex::new("Connecting to VNC...".to_string()));
        let status_clone = status_text.clone();

        let framebuffer = Arc::new(Mutex::new(VncFramebuffer::new(1024, 768)));
        let fb_clone = framebuffer.clone();

        thread::Builder::new()
            .name("vnc-worker".to_string())
            .spawn(move || {
                let addr = format!("{}:{}", host, port);
                let stream = match TcpStream::connect_timeout(
                    &addr.parse().unwrap_or_else(|_| {
                        use std::net::ToSocketAddrs;
                        addr.to_socket_addrs()
                            .ok()
                            .and_then(|mut i| i.next())
                            .unwrap_or_else(|| "127.0.0.1:5900".parse().unwrap())
                    }),
                    Duration::from_secs(10),
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        *status_clone.lock() = format!("Failed to connect: {}", e);
                        is_alive_clone.store(false, Ordering::Relaxed);
                        repaint_signal();
                        return;
                    }
                };

                let pass_copy = password.clone();
                let client_res = Client::from_tcp_stream(stream, true, move |methods| {
                    let has_none = methods.iter().any(|m| matches!(m, AuthMethod::None));
                    let has_password = methods.iter().any(|m| matches!(m, AuthMethod::Password));

                    if has_none {
                        Some(AuthChoice::None)
                    } else if has_password {
                        if let Some(ref pass) = pass_copy {
                            let mut key = [0u8; 8];
                            let bytes = pass.as_bytes();
                            let len = bytes.len().min(8);
                            key[..len].copy_from_slice(&bytes[..len]);
                            Some(AuthChoice::Password(key))
                        } else {
                            Some(AuthChoice::Password([0u8; 8]))
                        }
                    } else {
                        None
                    }
                });

                let mut client = match client_res {
                    Ok(c) => c,
                    Err(e) => {
                        *status_clone.lock() = format!("VNC Auth/Protocol error: {:?}", e);
                        is_alive_clone.store(false, Ordering::Relaxed);
                        repaint_signal();
                        return;
                    }
                };

                let (w, h) = client.size();
                fb_clone.lock().resize(w as usize, h as usize);
                *status_clone.lock() = format!("Connected ({}x{})", w, h);
                repaint_signal();

                // Request initial full screen update
                let full_rect = Rect {
                    left: 0,
                    top: 0,
                    width: w,
                    height: h,
                };
                let _ = client.request_update(full_rect, false);

                while is_alive_clone.load(Ordering::Relaxed) {
                    // Check user input actions
                    while let Ok(action) = action_rx.try_recv() {
                        match action {
                            VncAction::Pointer { buttons, x, y } => {
                                let _ = client.send_pointer_event(buttons, x, y);
                            }
                            VncAction::Key { down, key } => {
                                let _ = client.send_key_event(down, key);
                            }
                            VncAction::Close => {
                                let _ = client.disconnect();
                                is_alive_clone.store(false, Ordering::Relaxed);
                                return;
                            }
                        }
                    }

                    // Poll VNC events
                    let mut got_event = false;
                    while let Some(event) = client.poll_event() {
                        got_event = true;
                        match event {
                            Event::Resize(new_w, new_h) => {
                                fb_clone.lock().resize(new_w as usize, new_h as usize);
                                let _ = client.request_update(
                                    Rect {
                                        left: 0,
                                        top: 0,
                                        width: new_w,
                                        height: new_h,
                                    },
                                    false,
                                );
                                repaint_signal();
                            }
                            Event::PutPixels(rect, pixels) => {
                                fb_clone.lock().put_pixels(rect, &pixels);
                            }
                            Event::EndOfFrame => {
                                repaint_signal();
                                // Request incremental update
                                let sz = client.size();
                                let _ = client.request_update(
                                    Rect {
                                        left: 0,
                                        top: 0,
                                        width: sz.0,
                                        height: sz.1,
                                    },
                                    true,
                                );
                            }
                            Event::Disconnected(err) => {
                                *status_clone.lock() = format!("Disconnected: {:?}", err);
                                is_alive_clone.store(false, Ordering::Relaxed);
                                repaint_signal();
                                return;
                            }
                            _ => {}
                        }
                    }

                    if !got_event {
                        thread::sleep(Duration::from_millis(15));
                    }
                }
            })?;

        Ok(Self {
            framebuffer,
            action_tx,
            is_alive,
            status_text,
            title,
        })
    }

    pub fn send_pointer(&self, buttons: u8, x: u16, y: u16) {
        if self.is_alive.load(Ordering::Relaxed) {
            let _ = self.action_tx.send(VncAction::Pointer { buttons, x, y });
        }
    }

    pub fn send_key(&self, down: bool, key: u32) {
        if self.is_alive.load(Ordering::Relaxed) {
            let _ = self.action_tx.send(VncAction::Key { down, key });
        }
    }

    pub fn close(&self) {
        let _ = self.action_tx.send(VncAction::Close);
    }
}
