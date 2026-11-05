# Contribuir a RusIO

¡Gracias por querer ayudar! Los issues y los pull requests son bienvenidos, en español o en inglés.

## Antes de empezar

- Para cambios grandes (un plugin nuevo, cambios en el contrato `rusio-plugin-api`), abre primero un issue y lo
  conversamos.
- Un pull request debería hacer una sola cosa y venir con pruebas cuando sea razonable.

## Entorno

```powershell
cargo build --release
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --release
```

El formato lo fija `rustfmt.toml`. La integración continua ejecuta pruebas, `fmt`, `clippy` y `cargo-deny`.

## Reglas del proyecto

- **Secretos**: nunca escribas contraseñas ni claves en `connections.json`, en logs ni en mensajes de error.
  Los campos secretos de un plugin se declaran en `ProtocolDef::secrets` y van al almacén del sistema.
- **La UI nunca se bloquea**: el trabajo lento (red, disco, consultas) va en un hilo aparte; la sesión solo
  dibuja el estado que el hilo actualiza.
- **Pruebas contra servidores reales** (PostgreSQL, MySQL, MongoDB) van detrás de variables de entorno y se
  omiten solas si no existen; no las hagas obligatorias.
- **Dependencias**: solo licencias permisivas (MIT, Apache-2.0, BSD, ISC, Zlib…). `cargo-deny` lo comprueba.

## Un plugin nuevo

Sigue la sección «Cómo agregar un plugin» del [README](README.md). Para una base de datos basta implementar
`SqlBackend` y usar `SqlConsole`.

## Licencia de las contribuciones

Al enviar un pull request aceptas que tu contribución se licencia bajo MIT y Apache-2.0, igual que el resto del
proyecto.
