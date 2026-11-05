# ⚡ RusIO

**RusIO** es un cliente SSH, visor VNC y gestor de terminales locales moderno, ultrarrápido y desarrollado **100% en Rust**.

Diseñado para administradores de sistemas, desarrolladores y entusiastas de DevOps que buscan una alternativa nativa, liviana y sin el consumo de memoria de clientes basados en Electron.

---

## 🚀 Características Principales

- **Gestor de Conexiones Jerárquico (Sidebar)**:
  - Organización de conexiones en carpetas y subcarpetas ilimitadas.
  - Búsqueda y filtrado instantáneo en tiempo real.
  - Creación, edición, duplicación y eliminación de perfiles.
  - Badges de colores por protocolo: `[SSH]`, `[LOCAL]`, `[VNC]`.
  - Persistencia automática en `%APPDATA%/rusio/connections.json`.

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
El ejecutable final se genera en `target/release/rusio.exe` (~5.9 MB).

### Ejecutar Pruebas Unitarias
```powershell
cargo test
```

---

## 📁 Estructura del Proyecto

RusIO es un workspace de Cargo. La app no conoce ningún protocolo: solo habla con el
contrato de plugins (`rusio-plugin-api`) y cada protocolo vive en su propio crate.

```text
crates/
├── rusio-core/            # Sin UI: perfiles, ajustes, almacén de credenciales
├── rusio-plugin-api/      # El contrato (traits Plugin / Session / Host), tema, iconos
│                          #   el widget de terminal compartido (emulador VT100 + egui) y la grilla de resultados
#   tabulares (`data::data_grid`) que reutilizará cualquier plugin de base de datos
├── rusio-plugin-ssh/      # SSH (terminal) + SFTP (explorador y edición remota)
├── rusio-plugin-local/    # Terminales locales: PowerShell, CMD, WSL, Git Bash
├── rusio-plugin-vnc/      # Escritorio remoto VNC (RFB)
├── rusio-plugin-sqlite/   # Bases SQLite: esquema, editor SQL, resultados y export a CSV
└── rusio-app/             # El binario `rusio`: ventana, pestañas, sidebar, ajustes, editor
```

### Cómo agregar un plugin

1. Crea `crates/rusio-plugin-<nombre>` con dependencia a `rusio-plugin-api`.
2. Implementa el trait `Plugin`:
   - `protocol()` → id (clave en `connections.json`), nombre, badge, color y qué campos de los
     parámetros son secretos (van al almacén de credenciales, nunca al archivo).
   - `default_params()`, `summary()` y `edit_ui()` → los parámetros de conexión viajan como
     `serde_json::Value`; el plugin los (de)serializa a su propio struct y dibuja su parte del
     formulario con egui.
   - `settings_schema()` → ajustes propios; aparecen en la sección del plugin dentro de
     Configuración y llegan de vuelta en `ConnectRequest::settings`.
   - `connect()` → devuelve un `Session` que dibuja su propia pestaña (`Session::ui`).
     Las sesiones pueden pedir al host abrir un archivo en el editor de texto (`Host`).
   - Túneles (opcional): `ProtocolDef::endpoint` permite que una conexión de ese protocolo se
     alcance *a través de* una puerta de enlace (el host lee y reescribe su host/puerto);
     `ProtocolDef::gateway` + `Plugin::open_tunnel()` permiten que el plugin *sea* esa puerta
     (SSH lo es). Una conexión guarda `via: <id de otra conexión>`; el host encadena hasta
     5 puertas, abre un túnel por cada una en un hilo aparte y mantiene los túneles abiertos
     mientras viva la pestaña.
3. Regístralo en `crates/rusio-app/src/registry.rs` detrás de una *feature* de Cargo
   (`default = ["ssh", "local", "vnc", "sqlite"]`), así cada build elige qué plugins incluye:
   `cargo build -p rusio --no-default-features --features ssh`.

Los plugins entran por pull request al repositorio y se compilan junto con la app: comparten
proceso y pueden dibujar con egui, y la revisión de código es la barrera de seguridad.

### Formato de `connections.json`

El protocolo se guarda como `{ "<id del plugin>": { ...parámetros... } }`, el mismo formato
que usaban las versiones anteriores a los plugins, así que los archivos existentes siguen
funcionando.
