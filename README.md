# RusIO

**RusIO** (*Rust IO*) es un gestor de conexiones nativo, rápido y escrito 100 % en Rust: terminales SSH y locales,
explorador SFTP, escritorio remoto VNC y consolas de bases de datos, todo en una sola ventana con pestañas.

El nombre es un guiño a *Input/Output* y a Rucio, un gato.

[![Installer](https://github.com/Fiambre/RusIO/actions/workflows/release.yml/badge.svg)](https://github.com/Fiambre/RusIO/actions/workflows/release.yml)
[![CI](https://github.com/Fiambre/RusIO/actions/workflows/ci.yml/badge.svg)](https://github.com/Fiambre/RusIO/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#licencia)

## Instalación

Descarga `RusIO-Setup-<versión>.exe` desde la página de [Releases](https://github.com/Fiambre/RusIO/releases).
Se instala por usuario, **sin permisos de administrador**, en `%LOCALAPPDATA%\Programs\RusIO`, y crea un acceso
en el menú Inicio. Al desinstalar no se borran tus conexiones ni tus contraseñas.

> Los instaladores aún no están firmados digitalmente, así que Windows SmartScreen puede mostrar una advertencia
> la primera vez ("Más información" → "Ejecutar de todos modos").

### Actualizaciones

Al iniciar, RusIO consulta la última release pública de GitHub. Si hay una versión nueva, aparece un aviso en la
barra de estado; con **Actualizar ahora** descarga el instalador, verifica su firma, se cierra, instala la versión
nueva y se vuelve a abrir. Las conexiones y contraseñas no se tocan. Se puede desactivar en
Ajustes → Actualizaciones.

Cada instalador publicado lleva una firma Ed25519 (`RusIO-Setup-<versión>.exe.sig`) hecha por el workflow de
release con una clave privada que solo existe como secreto del repositorio. La clave pública está dentro de la
app (`crates/rusio-app/src/updates.rs`): **si la firma no coincide, el instalador no se ejecuta**. Esto protege
contra un archivo alterado o sustituido en la release, pero no sustituye a una firma de código de Windows.

Por ahora solo se publica para Windows 10/11 (x64). El código no depende de Windows salvo los terminales locales
(ConPTY), así que otras plataformas son posibles; las contribuciones son bienvenidas.

## Características

- **Conexiones organizadas**: carpetas, búsqueda instantánea, iconos configurables por conexión, doble clic para
  conectar y menú contextual para editar o borrar. Las categorías (Local, Remoto, Bases de datos) guían la
  pantalla de inicio y el asistente de nueva conexión.
- **SSH**: terminal `xterm-256color` con autenticación por contraseña, clave privada (con passphrase) o agente.
- **SFTP**: explorador de archivos con subida y descarga, arrastrar y soltar, creación y borrado, y un editor de
  texto para archivos remotos.
- **Túneles y saltos**: cualquier conexión puede alcanzarse *a través de* otra conexión SSH (hasta 5 saltos
  encadenados), por ejemplo una base de datos tras un bastión.
- **Terminales locales**: PowerShell, CMD, WSL y Git Bash sobre ConPTY.
- **VNC**: cliente RFB con ratón, teclado y `Ctrl+Alt+Del`.
- **Bases de datos**, con una consola SQL común (explorador de esquema, editor con colores, numeración de líneas y
  autocompletado de tablas y columnas, resultados en cuadrícula, exportación a CSV, formateador `Ctrl+Mayús+F`
  y cancelación de consultas):
  - SQLite
  - PostgreSQL (con selector de base de datos)
  - MySQL / MariaDB
  - MongoDB, con sintaxis de shell (`db.coleccion.find({...}).sort({...}).limit(10)`, `aggregate`,
    `insertOne`, `updateMany`, etc.)
- **Seguridad**:
  - Las contraseñas se guardan en el **almacén de credenciales del sistema** (Windows Credential Manager), nunca
    en `connections.json`.
  - Las consultas que pueden destruir datos (`DROP`, `TRUNCATE`, `DELETE`/`UPDATE` sin `WHERE`, `deleteMany`…)
    piden confirmación antes de ejecutarse.
  - Las conexiones de bases de datos admiten modo de solo lectura.
- **Interfaz propia**: barra de título personalizada, tema oscuro, barra de estado y ajustes por plugin.

## Datos y privacidad

- Las conexiones se guardan en `connections.json` dentro de la carpeta de configuración del usuario
  (`%APPDATA%\rusio` en Windows). La ubicación se puede cambiar en Ajustes.
- Las contraseñas y las URI con credenciales no se escriben en ese archivo: van al almacén del sistema, con el
  servicio `rusio`.
- RusIO no envía telemetría. Aparte de los servidores que configures tú, solo hace una consulta al iniciar a la
  API pública de GitHub para saber si hay una versión nueva (no envía datos tuyos). Solo descarga e instala algo
  si tú pulsas **Actualizar ahora**. Se puede desactivar en Ajustes → Actualizaciones.

## Compilar desde el código

Requisitos: Rust estable reciente (MSVC en Windows) y, para el instalador, [NSIS](https://nsis.sourceforge.io).

```powershell
cargo run                      # modo debug
cargo build --release          # target/release/rusio.exe
cargo test --workspace         # pruebas unitarias
```

Cada plugin es una *feature* de Cargo (`ssh`, `local`, `vnc`, `sqlite`, `postgres`, `mysql`, `mongodb`, todas
activas por defecto). Para compilar solo algunos:

```powershell
cargo build --release -p rusio --no-default-features --features "ssh,postgres"
```

Instalador (se genera en `dist/`):

```powershell
makensis -DVERSION=0.1.0 installer/rusio.nsi
```

Las pruebas contra servidores reales (PostgreSQL, MySQL, MongoDB) se omiten solas salvo que definas
`RUSIO_PG_TEST`, `RUSIO_MYSQL_TEST` o `RUSIO_MONGO_TEST`; mira `engine.rs` de cada plugin para el formato.

## Arquitectura

RusIO es un workspace de Cargo. La app no conoce ningún protocolo: solo habla con el contrato de plugins
(`rusio-plugin-api`) y cada protocolo vive en su propio crate.

```text
crates/
├── rusio-core/             # Sin UI: perfiles, ajustes, almacén de credenciales
├── rusio-plugin-api/       # El contrato (Plugin / Session / Host), tema, iconos, terminal compartido,
│                           #   cuadrícula de datos y la consola SQL común (SqlConsole / SqlBackend)
├── rusio-plugin-ssh/       # SSH (terminal) + SFTP (explorador y edición remota)
├── rusio-plugin-local/     # Terminales locales: PowerShell, CMD, WSL, Git Bash
├── rusio-plugin-vnc/       # Escritorio remoto VNC (RFB)
├── rusio-plugin-sqlite/    # SQLite
├── rusio-plugin-postgres/  # PostgreSQL
├── rusio-plugin-mysql/     # MySQL / MariaDB
├── rusio-plugin-mongodb/   # MongoDB (sintaxis de shell)
└── rusio-app/              # El binario `rusio`: ventana, pestañas, barra lateral, ajustes, editor
```

### Cómo agregar un plugin

1. Crea `crates/rusio-plugin-<nombre>` con dependencia a `rusio-plugin-api`.
2. Implementa el trait `Plugin`:
   - `protocol()` → id (clave en `connections.json`), nombre, badge, categoría, icono (logo SVG o glifo), color
     y qué campos de los parámetros son secretos (van al almacén de credenciales, nunca al archivo).
   - `default_params()`, `summary()` y `edit_ui()` → los parámetros de conexión viajan como
     `serde_json::Value`; el plugin los (de)serializa a su propio struct y dibuja su parte del formulario.
   - `settings_schema()` → ajustes propios; aparecen en la sección del plugin dentro de Ajustes.
   - `connect()` → devuelve un `Session` que dibuja su propia pestaña. Las sesiones pueden pedir al host abrir
     un archivo en el editor de texto.
   - Túneles (opcional): `ProtocolDef::endpoint` permite que una conexión se alcance *a través de* una puerta de
     enlace; `ProtocolDef::gateway` + `Plugin::open_tunnel()` permiten que el plugin *sea* esa puerta (SSH lo es).
3. Para una base de datos basta implementar `SqlBackend` y envolverlo en `SqlConsole`: la consola, el editor, el
   autocompletado y la exportación vienen incluidos.
4. Regístralo en `crates/rusio-app/src/registry.rs` detrás de una *feature* de Cargo.

Los plugins se compilan junto con la app: comparten proceso y pueden dibujar con egui, y la revisión de código
en los pull requests es la barrera de seguridad. Mira [CONTRIBUTING.md](CONTRIBUTING.md).

### Formato de `connections.json`

El protocolo se guarda como `{ "<id del plugin>": { ...parámetros... } }`. Los campos secretos quedan en blanco
en el archivo y se leen del almacén de credenciales del sistema.

## Créditos y marcas

- Interfaz con [egui](https://github.com/emilk/egui). Las dependencias y sus licencias están en `Cargo.lock`;
  todas son permisivas y se verifican en CI con `cargo-deny`.
- Los logos de SQLite, PostgreSQL, MySQL y MongoDB provienen de [Simple Icons](https://simpleicons.org)
  (licencia CC0). Las marcas pertenecen a sus respectivos dueños y se usan solo para identificar cada tipo de
  base de datos; RusIO no está afiliado ni respaldado por ellos.

## Licencia

Doble licencia, a tu elección:

- [Apache License 2.0](LICENSE-APACHE)
- [MIT](LICENSE-MIT)

Salvo que indiques lo contrario, cualquier contribución que envíes para incluirla en RusIO tendrá esa misma doble
licencia, sin términos adicionales.
