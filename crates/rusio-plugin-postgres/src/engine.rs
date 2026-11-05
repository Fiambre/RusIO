//! PostgreSQL access: connecting, running statements and reading the schema, plus a worker
//! thread that owns the connection so the UI never blocks on a query.
//!
//! Statements go through the simple query protocol, so every value arrives as text and
//! any column type can be shown without a per-type decoder.
use crossbeam_channel::{unbounded, Receiver, Sender};
use parking_lot::Mutex;
use postgres::config::SslMode;
use postgres::{Client, Config, NoTls, SimpleQueryMessage};
use postgres_native_tls::MakeTlsConnector;
use rusio_plugin_api::data::DataTable;
use rusio_plugin_api::sql::{ConsoleState, QueryResult, SchemaTable, SqlBackend};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ssl {
    Disable,
    /// TLS if the server offers it, plain otherwise.
    Prefer,
    /// TLS or fail. Like libpq's `require`, the certificate is not verified.
    Require,
}

impl Ssl {
    pub fn parse(s: &str) -> Ssl {
        match s {
            "disable" => Ssl::Disable,
            "require" => Ssl::Require,
            _ => Ssl::Prefer,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConnectOptions {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Option<String>,
    pub database: String,
    pub ssl: Ssl,
    pub read_only: bool,
    pub connect_timeout_secs: u64,
    /// 0 = no limit.
    pub statement_timeout_secs: u64,
}

/// Asks the server to abort the running query (from another connection).
pub type Canceller = Box<dyn Fn() + Send + Sync>;

pub fn connect(opts: &ConnectOptions) -> Result<(Client, Canceller), String> {
    if opts.host.trim().is_empty() {
        return Err("Indica el servidor.".to_string());
    }
    let mut cfg = Config::new();
    cfg.host(&opts.host)
        .port(opts.port)
        .user(&opts.user)
        .dbname(&opts.database)
        .connect_timeout(Duration::from_secs(opts.connect_timeout_secs.max(1)))
        .application_name("RusIO");
    if let Some(pw) = &opts.password {
        cfg.password(pw);
    }
    cfg.ssl_mode(match opts.ssl {
        Ssl::Disable => SslMode::Disable,
        Ssl::Prefer => SslMode::Prefer,
        Ssl::Require => SslMode::Require,
    });

    let (mut client, canceller): (Client, Canceller) = if opts.ssl == Ssl::Disable {
        let client = cfg.connect(NoTls).map_err(describe)?;
        let token = client.cancel_token();
        (client, Box::new(move || drop(token.cancel_query(NoTls))))
    } else {
        let tls = native_tls::TlsConnector::builder()
            .danger_accept_invalid_certs(true)
            .danger_accept_invalid_hostnames(true)
            .build()
            .map_err(|e| format!("No se pudo preparar TLS: {e}"))?;
        let tls = MakeTlsConnector::new(tls);
        let client = cfg.connect(tls.clone()).map_err(describe)?;
        let token = client.cancel_token();
        (client, Box::new(move || drop(token.cancel_query(tls.clone()))))
    };

    if opts.read_only {
        client
            .batch_execute("SET default_transaction_read_only = on")
            .map_err(describe)?;
    }
    if opts.statement_timeout_secs > 0 {
        client
            .batch_execute(&format!(
                "SET statement_timeout = {}",
                opts.statement_timeout_secs * 1000
            ))
            .map_err(describe)?;
    }
    Ok((client, canceller))
}

/// Server errors read best as "ERROR: message (detail)"; the rest fall back to the chain.
fn describe(e: postgres::Error) -> String {
    if let Some(db) = e.as_db_error() {
        let mut s = format!("{}: {}", db.severity(), db.message());
        if let Some(d) = db.detail() {
            s.push_str(&format!(" ({d})"));
        }
        if let Some(h) = db.hint() {
            s.push_str(&format!(" Sugerencia: {h}"));
        }
        return s;
    }
    let mut s = e.to_string();
    let mut src = std::error::Error::source(&e);
    while let Some(inner) = src {
        s.push_str(&format!(": {inner}"));
        src = inner.source();
    }
    s
}

/// Databases this role can connect to.
pub fn list_databases(client: &mut Client) -> Vec<String> {
    let Ok(msgs) = client.simple_query(
        "SELECT datname FROM pg_database \
         WHERE NOT datistemplate AND has_database_privilege(datname, 'CONNECT') ORDER BY datname",
    ) else {
        return Vec::new();
    };
    msgs.into_iter()
        .filter_map(|m| match m {
            SimpleQueryMessage::Row(r) => r.get(0).map(str::to_string),
            _ => None,
        })
        .collect()
}

pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Runs every statement in `sql`; the result is that of the last one.
pub fn run_sql(client: &mut Client, sql: &str, max_rows: usize) -> Result<QueryResult, String> {
    if sql.trim().is_empty() {
        return Err("Escribe una consulta.".to_string());
    }
    let start = Instant::now();
    let messages = client.simple_query(sql).map_err(describe)?;

    let mut cur: Option<(DataTable, bool)> = None;
    let mut last: Option<(DataTable, bool)> = None;
    let mut changed = 0u64;
    let mut statements = 0usize;

    for m in messages {
        match m {
            SimpleQueryMessage::RowDescription(cols) => {
                cur = Some((
                    DataTable {
                        columns: cols.iter().map(|c| c.name().to_string()).collect(),
                        rows: Vec::new(),
                    },
                    false,
                ));
            }
            SimpleQueryMessage::Row(row) => {
                let (table, truncated) = cur.get_or_insert_with(|| {
                    (
                        DataTable {
                            columns: row.columns().iter().map(|c| c.name().to_string()).collect(),
                            rows: Vec::new(),
                        },
                        false,
                    )
                });
                if table.rows.len() >= max_rows {
                    *truncated = true;
                } else {
                    table.rows.push(
                        (0..row.len())
                            .map(|i| row.get(i).map_or_else(|| "NULL".to_string(), str::to_string))
                            .collect(),
                    );
                }
            }
            SimpleQueryMessage::CommandComplete(n) => {
                statements += 1;
                match cur.take() {
                    Some(t) => last = Some(t),
                    None => {
                        last = None;
                        changed = n;
                    }
                }
            }
            _ => {}
        }
    }
    if statements == 0 {
        return Err("No hay ninguna sentencia SQL que ejecutar.".to_string());
    }

    let elapsed_ms = start.elapsed().as_millis();
    Ok(match last {
        Some((table, truncated)) => {
            let n = table.rows.len();
            let message = if truncated {
                format!("{n} filas (resultado limitado)")
            } else if n == 1 {
                "1 fila".to_string()
            } else {
                format!("{n} filas")
            };
            QueryResult {
                table: Some(table),
                truncated,
                elapsed_ms,
                message,
            }
        }
        None => QueryResult {
            table: None,
            truncated: false,
            elapsed_ms,
            message: format!("{changed} fila(s) afectada(s)"),
        },
    })
}

/// Tables and views outside the system schemas. Names are `schema.table`, except `public`.
///
/// Reads `pg_catalog` rather than `information_schema`, which hides every table the user has
/// no privilege on (so a read-only role would see an empty database).
pub fn load_schema(client: &mut Client) -> Result<Vec<SchemaTable>, String> {
    const FILTER: &str = "c.relkind IN ('r','p','v','m','f') \
        AND n.nspname NOT IN ('pg_catalog','information_schema') \
        AND n.nspname NOT LIKE 'pg_toast%' AND n.nspname NOT LIKE 'pg_temp%'";
    let rows = |client: &mut Client, sql: &str| -> Result<Vec<Vec<String>>, String> {
        let mut out = Vec::new();
        for m in client.simple_query(sql).map_err(describe)? {
            if let SimpleQueryMessage::Row(r) = m {
                out.push((0..r.len()).map(|i| r.get(i).unwrap_or_default().to_string()).collect());
            }
        }
        Ok(out)
    };

    let tables = rows(
        client,
        &format!(
            "SELECT n.nspname, c.relname, CASE WHEN c.relkind IN ('v','m') THEN 'VIEW' ELSE 'BASE TABLE' END \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE {FILTER} ORDER BY n.nspname, c.relname"
        ),
    )?;
    let columns = rows(
        client,
        &format!(
            "SELECT n.nspname, c.relname, a.attname, format_type(a.atttypid, a.atttypmod) \
             FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE a.attnum > 0 AND NOT a.attisdropped AND {FILTER} \
             ORDER BY n.nspname, c.relname, a.attnum"
        ),
    )?;

    let mut by_table: HashMap<(String, String), Vec<(String, String)>> = HashMap::new();
    for c in columns {
        by_table
            .entry((c[0].clone(), c[1].clone()))
            .or_default()
            .push((c[2].clone(), c[3].clone()));
    }
    Ok(tables
        .into_iter()
        .map(|t| {
            let cols = by_table.remove(&(t[0].clone(), t[1].clone())).unwrap_or_default();
            SchemaTable {
                name: display_name(&t[0], &t[1]),
                is_view: t[2] == "VIEW",
                columns: cols,
            }
        })
        .collect())
}

/// For an empty schema: the databases this role can connect to, since the usual mistake is
/// having connected to the default `postgres` database instead of the one with the data.
pub fn databases_hint(client: &mut Client, current: &str, selector: bool) -> Option<QueryResult> {
    let msgs = client
        .simple_query(
            "SELECT datname AS base_de_datos FROM pg_database \
             WHERE NOT datistemplate AND has_database_privilege(datname, 'CONNECT') ORDER BY datname",
        )
        .ok()?;
    let rows: Vec<Vec<String>> = msgs
        .into_iter()
        .filter_map(|m| match m {
            SimpleQueryMessage::Row(r) => Some(vec![r.get(0).unwrap_or_default().to_string()]),
            _ => None,
        })
        .collect();
    let how = if selector {
        "elígela en el selector de la izquierda"
    } else {
        "edita la conexión y cambia \"Base de datos\" (o activa \"Mostrar todas las bases de datos\")"
    };
    Some(QueryResult {
        message: format!(
            "La base \"{current}\" no tiene tablas. Estas son las bases a las que puedes conectarte; \
             para usar otra, {how}:"
        ),
        table: Some(DataTable {
            columns: vec!["base_de_datos".to_string()],
            rows,
        }),
        truncated: false,
        elapsed_ms: 0,
    })
}

fn display_name(schema: &str, table: &str) -> String {
    if schema == "public" {
        table.to_string()
    } else {
        format!("{schema}.{table}")
    }
}

/// Inverse of [`display_name`] over the schema we loaded; unknown names are taken as `public`.
fn select_all_for(known: &HashMap<String, (String, String)>, name: &str) -> String {
    let (schema, table) = known
        .get(name)
        .cloned()
        .unwrap_or_else(|| ("public".to_string(), name.to_string()));
    format!(
        "SELECT * FROM {}.{} LIMIT 100;",
        quote_ident(&schema),
        quote_ident(&table)
    )
}

pub enum Cmd {
    Run(String),
    RefreshSchema,
    Switch(String),
    Close,
}

/// Handle the UI keeps: the worker owns the connection.
pub struct Engine {
    cmd_tx: Sender<Cmd>,
    state: Arc<Mutex<ConsoleState>>,
    alive: Arc<AtomicBool>,
    /// Replaced when the worker reconnects to another database.
    canceller: Arc<Mutex<Canceller>>,
    /// Listed in the console's selector (empty = no selector).
    dbs: Arc<Mutex<Vec<String>>>,
    current: Arc<Mutex<String>>,
    /// Display name -> (schema, table), refreshed with the schema.
    names: Arc<Mutex<HashMap<String, (String, String)>>>,
}

fn apply_schema(
    schema: Result<Vec<SchemaTable>, String>,
    state: &Mutex<ConsoleState>,
    names: &Mutex<HashMap<String, (String, String)>>,
) {
    let mut s = state.lock();
    match schema {
        Ok(t) => {
            // Display names are `schema.table` or a bare `table` (public): rebuild the lookup.
            let mut map = names.lock();
            map.clear();
            for t in &t {
                let (schema, table) = match t.name.split_once('.') {
                    Some((a, b)) => (a.to_string(), b.to_string()),
                    None => ("public".to_string(), t.name.clone()),
                };
                map.insert(t.name.clone(), (schema, table));
            }
            s.tables = t;
        }
        Err(e) => s.error = Some(e),
    }
}

fn show_hint_if_empty(client: &mut Client, database: &str, selector: bool, state: &Mutex<ConsoleState>) {
    let empty = {
        let s = state.lock();
        s.tables.is_empty() && s.error.is_none()
    };
    if empty {
        if let Some(hint) = databases_hint(client, database, selector) {
            state.lock().result = Some(hint);
        }
    }
}

impl Engine {
    pub fn start(
        mut client: Client,
        canceller: Canceller,
        opts: ConnectOptions,
        all_databases: bool,
        max_rows: usize,
        repaint: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (cmd_tx, cmd_rx): (Sender<Cmd>, Receiver<Cmd>) = unbounded();
        let state = Arc::new(Mutex::new(ConsoleState::default()));
        let alive = Arc::new(AtomicBool::new(true));
        let names = Arc::new(Mutex::new(HashMap::new()));

        let dbs = Arc::new(Mutex::new(if all_databases {
            list_databases(&mut client)
        } else {
            Vec::new()
        }));
        let current = Arc::new(Mutex::new(opts.database.clone()));
        let canceller = Arc::new(Mutex::new(canceller));

        apply_schema(load_schema(&mut client), &state, &names);
        show_hint_if_empty(&mut client, &opts.database, all_databases, &state);

        let (st, al, nm) = (state.clone(), alive.clone(), names.clone());
        let (dbs_w, cur_w, canc_w) = (dbs.clone(), current.clone(), canceller.clone());
        thread::Builder::new()
            .name("postgres-worker".to_string())
            .spawn(move || {
                while let Ok(cmd) = cmd_rx.recv() {
                    match cmd {
                        Cmd::Close => break,
                        Cmd::RefreshSchema => {
                            apply_schema(load_schema(&mut client), &st, &nm);
                            if all_databases {
                                *dbs_w.lock() = list_databases(&mut client);
                            }
                            let db = cur_w.lock().clone();
                            show_hint_if_empty(&mut client, &db, all_databases, &st);
                        }
                        Cmd::Switch(db) => {
                            st.lock().busy = true;
                            repaint();
                            let mut o = opts.clone();
                            o.database = db.clone();
                            match connect(&o) {
                                Ok((c, canc)) => {
                                    client = c;
                                    *canc_w.lock() = canc;
                                    *cur_w.lock() = db.clone();
                                    {
                                        let mut s = st.lock();
                                        s.result = None;
                                        s.error = None;
                                        s.tables.clear();
                                    }
                                    apply_schema(load_schema(&mut client), &st, &nm);
                                    show_hint_if_empty(&mut client, &db, all_databases, &st);
                                }
                                Err(e) => st.lock().error = Some(e),
                            }
                            st.lock().busy = false;
                        }
                        Cmd::Run(sql) => {
                            st.lock().busy = true;
                            repaint();
                            let outcome = run_sql(&mut client, &sql, max_rows);
                            // DDL/DML may have changed the schema.
                            let changed_schema = matches!(&outcome, Ok(r) if r.table.is_none());
                            if changed_schema {
                                apply_schema(load_schema(&mut client), &st, &nm);
                            }
                            let mut s = st.lock();
                            s.busy = false;
                            match outcome {
                                Ok(r) => {
                                    s.result = Some(r);
                                    s.error = None;
                                }
                                Err(e) => {
                                    s.error = Some(if e.contains("user request") {
                                        "Consulta cancelada.".to_string()
                                    } else {
                                        e
                                    });
                                }
                            }
                            if client.is_closed() {
                                s.error = Some("Se perdió la conexión con el servidor.".to_string());
                                break;
                            }
                        }
                    }
                    repaint();
                }
                al.store(false, Ordering::Relaxed);
                repaint();
            })
            .expect("spawn postgres worker");

        Self {
            cmd_tx,
            state,
            alive,
            canceller,
            dbs,
            current,
            names,
        }
    }
}

impl SqlBackend for Engine {
    fn state(&self) -> Arc<Mutex<ConsoleState>> {
        self.state.clone()
    }
    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }
    fn run(&self, sql: String) {
        let _ = self.cmd_tx.send(Cmd::Run(sql));
    }
    fn refresh_schema(&self) {
        let _ = self.cmd_tx.send(Cmd::RefreshSchema);
    }
    fn cancel(&self) {
        // Blocks on a network round trip; keep it off the UI thread.
        let c = self.canceller.clone();
        thread::spawn(move || (*c.lock())());
    }
    fn close(&self) {
        let _ = self.cmd_tx.send(Cmd::Close);
        self.alive.store(false, Ordering::Relaxed);
    }
    fn select_all_sql(&self, table: &str) -> String {
        select_all_for(&self.names.lock(), table)
    }
    fn databases(&self) -> Vec<String> {
        self.dbs.lock().clone()
    }
    fn current_database(&self) -> String {
        self.current.lock().clone()
    }
    fn switch_database(&self, name: &str) {
        let _ = self.cmd_tx.send(Cmd::Switch(name.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_quoted() {
        assert_eq!(quote_ident("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn select_all_resolves_schema_and_defaults_to_public() {
        let mut known = HashMap::new();
        known.insert(
            "ventas.pedido".to_string(),
            ("ventas".to_string(), "pedido".to_string()),
        );
        assert_eq!(
            select_all_for(&known, "ventas.pedido"),
            "SELECT * FROM \"ventas\".\"pedido\" LIMIT 100;"
        );
        assert_eq!(
            select_all_for(&known, "usuario"),
            "SELECT * FROM \"public\".\"usuario\" LIMIT 100;"
        );
    }

    #[test]
    fn ssl_mode_parsing_defaults_to_prefer() {
        assert_eq!(Ssl::parse("disable"), Ssl::Disable);
        assert_eq!(Ssl::parse("require"), Ssl::Require);
        assert_eq!(Ssl::parse("lo-que-sea"), Ssl::Prefer);
    }

    // ---- against a live server: set RUSIO_PG_TEST="host:port:user:password:database"
    fn live() -> Option<ConnectOptions> {
        let v = std::env::var("RUSIO_PG_TEST").ok()?;
        let p: Vec<&str> = v.split(':').collect();
        Some(ConnectOptions {
            host: p[0].to_string(),
            port: p[1].parse().ok()?,
            user: p[2].to_string(),
            password: (!p[3].is_empty()).then(|| p[3].to_string()),
            database: p[4].to_string(),
            ssl: Ssl::Prefer,
            read_only: false,
            connect_timeout_secs: 5,
            statement_timeout_secs: 0,
        })
    }

    fn wait_for(state: &Mutex<ConsoleState>, what: &str, mut cond: impl FnMut(&ConsoleState) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if cond(&state.lock()) {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {what}");
    }

    #[test]
    fn live_select_dml_schema_and_errors() {
        let Some(opts) = live() else { return };
        let (mut c, _) = connect(&opts).unwrap();
        c.batch_execute(
            "DROP SCHEMA IF EXISTS rusio_t CASCADE; CREATE SCHEMA rusio_t;
             CREATE TABLE rusio_t.persona(id serial PRIMARY KEY, nombre text, edad numeric, nac date);
             INSERT INTO rusio_t.persona(nombre, edad, nac) VALUES ('Ana', 30.5, '1990-01-02'), ('Luis', NULL, NULL);
             CREATE VIEW rusio_t.v AS SELECT nombre FROM rusio_t.persona;",
        )
        .unwrap();

        let r = run_sql(&mut c, "SELECT * FROM rusio_t.persona ORDER BY id", 100).unwrap();
        let t = r.table.unwrap();
        assert_eq!(t.columns, ["id", "nombre", "edad", "nac"]);
        assert_eq!(t.rows[0], ["1", "Ana", "30.5", "1990-01-02"]);
        assert_eq!(t.rows[1], ["2", "Luis", "NULL", "NULL"]);
        assert_eq!(r.message, "2 filas");

        let r = run_sql(&mut c, "SELECT * FROM rusio_t.persona", 1).unwrap();
        assert!(r.truncated);
        assert_eq!(r.table.unwrap().rows.len(), 1);

        // Empty result still reports its columns.
        let r = run_sql(&mut c, "SELECT nombre FROM rusio_t.persona WHERE false", 10).unwrap();
        assert_eq!(r.table.unwrap().columns, ["nombre"]);

        let r = run_sql(
            &mut c,
            "UPDATE rusio_t.persona SET edad = 1; SELECT count(*) AS n FROM rusio_t.persona",
            10,
        )
        .unwrap();
        assert_eq!(r.table.unwrap().rows, [["2"]]);

        let r = run_sql(&mut c, "DELETE FROM rusio_t.persona", 10).unwrap();
        assert!(r.table.is_none());
        assert_eq!(r.message, "2 fila(s) afectada(s)");

        let e = run_sql(&mut c, "SELECT * FROM rusio_t.no_existe", 10).unwrap_err();
        assert!(e.contains("no_existe"), "{e}");
        assert!(run_sql(&mut c, "   ", 10).is_err());
        // The connection survives an error.
        assert!(run_sql(&mut c, "SELECT 1", 10).is_ok());

        let schema = load_schema(&mut c).unwrap();
        let p = schema.iter().find(|t| t.name == "rusio_t.persona").unwrap();
        assert!(!p.is_view);
        assert_eq!(p.columns[1], ("nombre".to_string(), "text".to_string()));
        assert!(schema.iter().find(|t| t.name == "rusio_t.v").unwrap().is_view);

        c.batch_execute("DROP SCHEMA rusio_t CASCADE").unwrap();
    }

    #[test]
    fn live_empty_database_lists_the_available_ones() {
        let Some(opts) = live() else { return };
        let (mut c, _) = connect(&opts).unwrap();
        let hint = databases_hint(&mut c, "postgres", false).unwrap();
        assert!(hint.message.contains("no tiene tablas"));
        let rows = hint.table.unwrap().rows;
        assert!(rows.iter().any(|r| r[0] == "postgres"));
    }

    #[test]
    fn live_wrong_password_and_read_only() {
        let Some(mut opts) = live() else { return };
        let mut bad = opts.clone();
        bad.password = Some("clave-incorrecta-xyz".to_string());
        // Set RUSIO_PG_AUTH when the server really checks the password (not `trust`).
        match connect(&bad) {
            Err(e) => assert!(e.contains("password") || e.contains("contraseña"), "{e}"),
            Ok(_) => assert!(std::env::var("RUSIO_PG_AUTH").is_err(), "wrong password was accepted"),
        }

        opts.read_only = true;
        let (mut c, _) = connect(&opts).unwrap();
        let e = run_sql(&mut c, "CREATE TABLE rusio_ro(x int)", 10).unwrap_err();
        assert!(e.to_lowercase().contains("read-only"), "{e}");
    }

    #[test]
    fn live_worker_runs_and_cancels() {
        let Some(opts) = live() else { return };
        let (c, cancel) = connect(&opts).unwrap();
        let engine = Engine::start(c, cancel, opts.clone(), true, 100, || {});
        let state = engine.state();

        engine.run("SELECT 42 AS x".into());
        wait_for(&state, "select", |s| {
            !s.busy && s.result.as_ref().is_some_and(|r| r.message == "1 fila")
        });
        assert_eq!(
            state.lock().result.as_ref().unwrap().table.as_ref().unwrap().rows[0][0],
            "42"
        );

        engine.run("SELECT pg_sleep(30)".into());
        wait_for(&state, "start", |s| s.busy);
        engine.cancel();
        wait_for(&state, "cancel", |s| !s.busy && s.error.is_some());
        assert_eq!(state.lock().error.as_deref(), Some("Consulta cancelada."));

        // Still usable afterwards.
        engine.run("SELECT 1 AS y".into());
        wait_for(&state, "next", |s| s.error.is_none() && !s.busy && s.result.is_some());
        engine.close();
    }

    #[test]
    fn live_switching_database_reloads_schema() {
        let Some(opts) = live() else { return };
        let (mut c, _) = connect(&opts).unwrap();
        let _ = c.batch_execute("DROP DATABASE IF EXISTS rusio_t2");
        c.batch_execute("CREATE DATABASE rusio_t2").unwrap();
        let mut o2 = opts.clone();
        o2.database = "rusio_t2".to_string();
        let (mut c2, _) = connect(&o2).unwrap();
        c2.batch_execute("CREATE TABLE solo_en_t2(a int)").unwrap();
        drop(c2);

        let (c, cancel) = connect(&opts).unwrap();
        let engine = Engine::start(c, cancel, opts.clone(), true, 100, || {});
        let state = engine.state();
        assert!(engine.databases().contains(&"rusio_t2".to_string()));
        assert_eq!(engine.current_database(), opts.database);

        engine.switch_database("rusio_t2");
        wait_for(&state, "switch", |s| {
            !s.busy && s.tables.iter().any(|t| t.name == "solo_en_t2")
        });
        assert_eq!(engine.current_database(), "rusio_t2");

        engine.run("SELECT current_database()".into());
        wait_for(&state, "query", |s| {
            !s.busy
                && s.result
                    .as_ref()
                    .and_then(|r| r.table.as_ref())
                    .is_some_and(|t| t.rows[0][0] == "rusio_t2")
        });

        // A failed switch keeps the current connection.
        engine.switch_database("no_existe_xyz");
        wait_for(&state, "failed switch", |s| !s.busy && s.error.is_some());
        assert_eq!(engine.current_database(), "rusio_t2");
        engine.close();
        thread::sleep(Duration::from_millis(200));
        let (mut c, _) = connect(&opts).unwrap();
        c.batch_execute("DROP DATABASE rusio_t2").unwrap();
    }
}
