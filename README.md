# ⚡ Pulento

**Pulento** es un cliente SSH, visor VNC y gestor de terminales locales moderno, ultrarrápido y desarrollado **100% en Rust**.

Diseñado para administradores de sistemas, desarrolladores y entusiastas de DevOps que buscan una alternativa nativa, liviana y sin el consumo de memoria de clientes basados en Electron.

---

## 🚀 Características Principales

- **Gestor de Conexiones Jerárquico (Sidebar)**:
  - Organización de conexiones en carpetas y subcarpetas ilimitadas.
  - Búsqueda y filtrado instantáneo en tiempo real.
  - Creación, edición, duplicación y eliminación de perfiles.
  - Badges de colores por protocolo: `[SSH]`, `[LOCAL]`, `[VNC]`.
  - Persistencia automática en `%APPDATA%/pulento/connections.json`.

- **Soporte Multi-Protocolo**:
  - **SSH (Remoto)**:
    - Implementado con `ssh2` nativo.
    - Autenticación por contraseña, claves privadas (PEM/OpenSSH) con passphrase opcional, o SSH Agent de Windows.
    - Asignación de PTY `xterm-256color` con soporte de redimensionamiento dinámico.
  - **Terminales Locales (Windows ConPTY)**:
    - Implementado con `portable-pty`.
    - Soporte inmediato para **PowerShell**, **WSL (Linux Bash)**, **CMD** y shells personalizadas.
    - Entrada/salida asíncrona full duplex en hilo dedicado.
  - **VNC (Escritorio Remoto)**:
    - Cliente RFB/VNC integrado.
    - Renderizado de framebuffer acelerado por hardware a 60 FPS en egui.
    - Mapeo de eventos de mouse (clic primario, secundario, central) y teclado remoto.
    - Botón de combinación especial `Ctrl+Alt+Del`.

- **Gestión Avanzada de Pestañas (Tabs)**:
  - Múltiples sesiones activas simultáneamente en pestañas independientes.
  - Indicador de estado en tiempo real (🟢 Conectado / 🔴 Desconectado).
  - Título editable, selector de tamaño de fuente (`-` / `+`), botón de limpieza de pantalla.
  - Menú desplegable `➕` para abrir rápidamente PowerShell, WSL o conexiones rápidas.

- **Emulador de Terminal de Alto Rendimiento**:
  - Basado en el parser `vt100` en Rust.
  - Paleta ANSI completa de 16 colores, 256 colores y TrueColor (24-bit).
  - Agrupación inteligente de celdas contiguas (batching) para rendering a alta tasa de refresco.
  - Cursor interactivo y atajos de teclado estándar (`Ctrl+C`, `Ctrl+D`, `Ctrl+L`, etc.).

- **UI Moderna y Minimalista**:
  - Construido sobre `egui` / `eframe`.
  - Tema oscuro profesional inspirado en Warp y Termius.
  - Binario único de menos de 6 MB, sin dependencias externas.

---

## 🛠 Requisitos y Compilación

### Requisitos
- Rust 1.80+ (MSVC toolchain en Windows)
- Cargo

### Compilación en Modo Debug
```powershell
cargo run
```

### Compilación en Modo Release (Optimizado)
```powershell
cargo build --release
```
El ejecutable final se genera en `target/release/pulento.exe` (~5.9 MB).

### Ejecutar Pruebas Unitarias
```powershell
cargo test
```

---

## 📁 Estructura del Proyecto

```text
src/
├── main.rs               # Punto de entrada y configuración de ventana nativa
├── app.rs                # Estado global de la aplicación (PulentoApp)
├── config.rs             # Modelos de perfiles, carpetas y almacenamiento JSON
├── session.rs            # Abstracción de sesiones activas (Terminal / VNC)
├── theme.rs              # Paleta moderna oscura y estilos visuales
├── terminal/
│   ├── mod.rs            # Interfaz unificada de sesiones de terminal
│   ├── emulator.rs       # Emulador VT100 y renderizador monospace en egui
│   ├── local_session.rs  # ConPTY local (PowerShell, WSL Linux, CMD)
│   └── ssh_session.rs    # Cliente SSH2 interactivo multihilo
├── vnc/
│   ├── mod.rs            # Módulo VNC
│   └── session.rs        # Cliente RFB, framebuffer de píxeles y eventos
└── ui/
    ├── mod.rs            # Módulos UI
    ├── sidebar.rs        # Sidebar con carpetas y árbol de conexiones
    ├── tabs.rs           # Barra de pestañas y menú de creación rápida
    └── modal_conn.rs     # Modales para crear/editar conexiones y carpetas
```
