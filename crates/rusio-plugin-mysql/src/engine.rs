//! MySQL / MariaDB access: connecting, running statements and reading the schema, plus a
//! worker thread that owns the connection so the UI never blocks on a query.
//!
//! Statements use the text protocol, so every value arrives as text and any column type can
//! be shown without a per-type decoder.
use crossbeam_channel::{unbounded, Receiver, Sender};
use mysql::prelude::Queryable;
use mysql::{Conn, OptsBuilder, SslOpts, Value};
use parking_lot::Mutex;
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
    /// TLS if the server accepts it, plain otherwise.
    Prefer,
    /// TLS or fail. The certificate is not verified.
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
    /// Empty = no default database.
    pub database: String,
    pub ssl: Ssl,
    pub read_only: bool,
    pub connect_timeout_secs: u64,
    /// 0 = no limit.
    pub statement_timeout_secs: u64,
}

fn builder(o: &ConnectOptions, tls: bool) -> OptsBuilder {
    let b = OptsBuilder::new()
        .ip_or_hostname(Some(o.host.clone()))
        .tcp_port(o.port)
        .user(Some(o.user.clone()))
        .pass(o.password.clone())
        .db_name((!o.database.is_empty()).then(|| o.database.clone()))
        .tcp_connect_timeout(Some(Duration::from_secs(o.connect_timeout_secs.max(1))));
    if tls {
        b.ssl_opts(Some(
            SslOpts::default()
                .with_danger_accept_invalid_certs(true)
                .with_danger_skip_domain_validation(true),
        ))
    } else {
        b
    }
}

pub fn connect(o: &ConnectOptions) -> Result<Conn, String> {
    if o.host.trim().is_empty() {
        return Err("Indica el servidor.".to_string());
    }
    let mut conn = match o.ssl {
        Ssl::Disable => Conn::new(builder(o, false)),
        Ssl::Require => Conn::new(builder(o, true)),
        Ssl::Prefer => Conn::new(builder(o, true)).or_else(|_| Conn::new(builder(o, false))),
    }
    .map_err(|e| e.to_string())?;

    if o.read_only {
        conn.query_drop("SET SESSION TRANSACTION READ ONLY")
            .map_err(|e| e.to_string())?;
    }
    if o.statement_timeout_secs > 0 {
        // MySQL takes milliseconds, MariaDB seconds; whichever exists applies.
        let _ = conn.query_drop(format!(
            "SET SESSION max_execution_time = {}",
            o.statement_timeout_secs * 1000
        ));
        let _ = conn.query_drop(format!("SET SESSION max_statement_time = {}", o.statement_timeout_secs));
    }
    Ok(conn)
}

pub fn quote_ident(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

fn cell(v: &Value) -> String {
    match v {
        Value::NULL => "NULL".to_string(),
        Value::Bytes(b) => match std::str::from_utf8(b) {
            Ok(s) => s.to_string(),
            Err(_) => format!("<blob {} bytes>", b.len()),
        },
        Value::Int(i) => i.to_string(),
        Value::UInt(u) => u.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Double(d) => d.to_string(),
        Value::Date(y, mo, d, h, mi, s, us) => {
            if *h == 0 && *mi == 0 && *s == 0 && *us == 0 {
                format!("{y:04}-{mo:02}-{d:02}")
            } else {
                format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
            }
        }
        Value::Time(neg, d, h, mi, s, _) => {
            format!(
                "{}{:02}:{mi:02}:{s:02}",
                if *neg { "-" } else { "" },
                *d * 24 + u32::from(*h)
            )
        }
    }
}

/// Runs every statement in `sql`; the result is that of the last one.
pub fn run_sql(conn: &mut Conn, sql: &str, max_rows: usize) -> Result<QueryResult, String> {
    if sql.trim().is_empty() {
        return Err("Escribe una consulta.".to_string());
    }
    let start = Instant::now();
    let mut results = conn.query_iter(sql).map_err(|e| e.to_string())?;

    let mut last: Option<(DataTable, bool)> = None;
    let mut changed = 0u64;
    let mut statements = 0usize;

    while let Some(set) = results.iter() {
        statements += 1;
        let columns: Vec<String> = set
            .columns()
            .as_ref()
            .iter()
            .map(|c| c.name_str().to_string())
            .collect();
        if columns.is_empty() {
            changed = set.affected_rows();
            last = None;
            continue;
        }
        let mut rows = Vec::new();
        let mut truncated = false;
        // Rows past the limit are still read (the protocol requires it) but not kept.
        for row in set {
            let row = row.map_err(|e| e.to_string())?;
            if rows.len() >= max_rows {
                truncated = true;
                continue;
            }
            rows.push(
                (0..row.len())
                    .map(|i| row.as_ref(i).map_or_else(|| "NULL".to_string(), cell))
                    .collect(),
            );
        }
        last = Some((DataTable { columns, rows }, truncated));
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

/// Tables and views of the current database (none if no database is selected).
pub fn load_schema(conn: &mut Conn) -> Result<Vec<SchemaTable>, String> {
    let tables: Vec<(String, String)> = conn
        .query(
            "SELECT table_name, table_type FROM information_schema.tables \
             WHERE table_schema = DATABASE() ORDER BY table_name",
        )
        .map_err(|e| e.to_string())?;
    let columns: Vec<(String, String, String)> = conn
        .query(
            "SELECT table_name, column_name, column_type FROM information_schema.columns \
             WHERE table_schema = DATABASE() ORDER BY table_name, ordinal_position",
        )
        .map_err(|e| e.to_string())?;

    let mut by_table: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for (t, c, ty) in columns {
        by_table.entry(t).or_default().push((c, ty));
    }
    Ok(tables
        .into_iter()
        .map(|(name, kind)| SchemaTable {
            columns: by_table.remove(&name).unwrap_or_default(),
            is_view: kind.contains("VIEW"),
            name,
        })
        .collect())
}

pub fn list_databases(conn: &mut Conn) -> Vec<String> {
    conn.query::<String, _>("SHOW DATABASES").unwrap_or_default()
}

/// For an empty schema: the databases this user can see, since the usual mistake is having
/// connected without choosing the one that holds the data.
pub fn databases_hint(conn: &mut Conn, current: &str, selector: bool) -> Option<QueryResult> {
    let rows: Vec<Vec<String>> = list_databases(conn).into_iter().map(|d| vec![d]).collect();
    if rows.is_empty() {
        return None;
    }
    let what = if current.is_empty() {
        "No hay una base de datos seleccionada.".to_string()
    } else {
        format!("La base \"{current}\" no tiene tablas.")
    };
    let how = if selector {
        "elígela en el selector de la izquierda"
    } else {
        "edita la conexión y cambia \"Base de datos\" (o activa \"Mostrar todas las bases de datos\")"
    };
    Some(QueryResult {
        message: format!("{what} Estas son las bases que puedes ver; para usar una, {how}:"),
        table: Some(DataTable {
            columns: vec!["base_de_datos".to_string()],
            rows,
        }),
        truncated: false,
        elapsed_ms: 0,
    })
}

pub enum Cmd {
    Run(String),
    RefreshSchema,
    Switch(String),
    Close,
}

fn current_database(conn: &mut Conn) -> String {
    conn.query_first::<Option<String>, _>("SELECT DATABASE()")
        .ok()
        .flatten()
        .flatten()
        .unwrap_or_default()
}

fn apply_schema(schema: Result<Vec<SchemaTable>, String>, state: &Mutex<ConsoleState>) {
    let mut s = state.lock();
    match schema {
        Ok(t) => s.tables = t,
        Err(e) => s.error = Some(e),
    }
}

fn show_hint_if_empty(conn: &mut Conn, database: &str, selector: bool, state: &Mutex<ConsoleState>) {
    let empty = {
        let s = state.lock();
        s.tables.is_empty() && s.error.is_none()
    };
    if empty {
        if let Some(hint) = databases_hint(conn, database, selector) {
            state.lock().result = Some(hint);
        }
    }
}

/// Handle the UI keeps: the worker owns the connection.
pub struct Engine {
    cmd_tx: Sender<Cmd>,
    state: Arc<Mutex<ConsoleState>>,
    alive: Arc<AtomicBool>,
    opts: ConnectOptions,
    connection_id: u32,
    dbs: Arc<Mutex<Vec<String>>>,
    current: Arc<Mutex<String>>,
}

impl Engine {
    pub fn start(
        mut conn: Conn,
        opts: ConnectOptions,
        all_databases: bool,
        max_rows: usize,
        repaint: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (cmd_tx, cmd_rx): (Sender<Cmd>, Receiver<Cmd>) = unbounded();
        let state = Arc::new(Mutex::new(ConsoleState::default()));
        let alive = Arc::new(AtomicBool::new(true));
        let connection_id = conn.connection_id();
        let dbs = Arc::new(Mutex::new(if all_databases {
            list_databases(&mut conn)
        } else {
            Vec::new()
        }));
        let current = Arc::new(Mutex::new(current_database(&mut conn)));

        apply_schema(load_schema(&mut conn), &state);
        show_hint_if_empty(&mut conn, &current.lock().clone(), all_databases, &state);

        let (st, al, dbs_w, cur_w) = (state.clone(), alive.clone(), dbs.clone(), current.clone());
        thread::Builder::new()
            .name("mysql-worker".to_string())
            .spawn(move || {
                while let Ok(cmd) = cmd_rx.recv() {
                    match cmd {
                        Cmd::Close => break,
                        Cmd::RefreshSchema => {
                            apply_schema(load_schema(&mut conn), &st);
                            if all_databases {
                                *dbs_w.lock() = list_databases(&mut conn);
                            }
                            let db = cur_w.lock().clone();
                            show_hint_if_empty(&mut conn, &db, all_databases, &st);
                        }
                        Cmd::Switch(db) => {
                            st.lock().busy = true;
                            repaint();
                            match conn.query_drop(format!("USE {}", quote_ident(&db))) {
                                Ok(()) => {
                                    *cur_w.lock() = db.clone();
                                    {
                                        let mut s = st.lock();
                                        s.result = None;
                                        s.error = None;
                                        s.tables.clear();
                                    }
                                    apply_schema(load_schema(&mut conn), &st);
                                    show_hint_if_empty(&mut conn, &db, all_databases, &st);
                                }
                                Err(e) => st.lock().error = Some(e.to_string()),
                            }
                            st.lock().busy = false;
                        }
                        Cmd::Run(sql) => {
                            st.lock().busy = true;
                            repaint();
                            let outcome = run_sql(&mut conn, &sql, max_rows);
                            // DDL/DML or USE may have changed the schema or the current database.
                            let changed_schema = matches!(&outcome, Ok(r) if r.table.is_none());
                            if changed_schema {
                                *cur_w.lock() = current_database(&mut conn);
                                apply_schema(load_schema(&mut conn), &st);
                            }
                            let mut s = st.lock();
                            s.busy = false;
                            match outcome {
                                Ok(r) => {
                                    s.result = Some(r);
                                    s.error = None;
                                }
                                Err(e) => {
                                    s.error = Some(if e.contains("(1317)") || e.contains("interrupted") {
                                        "Consulta cancelada.".to_string()
                                    } else {
                                        e
                                    });
                                }
                            }
                            drop(s);
                            if conn.ping().is_err() {
                                st.lock().error = Some("Se perdió la conexión con el servidor.".to_string());
                                break;
                            }
                        }
                    }
                    repaint();
                }
                al.store(false, Ordering::Relaxed);
                repaint();
            })
            .expect("spawn mysql worker");

        Self {
            cmd_tx,
            state,
            alive,
            opts,
            connection_id,
            dbs,
            current,
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
        // Needs a second connection (KILL QUERY); keep the round trips off the UI thread.
        let (opts, id) = (self.opts.clone(), self.connection_id);
        thread::spawn(move || {
            if let Ok(mut c) = connect(&ConnectOptions {
                read_only: false,
                statement_timeout_secs: 0,
                ..opts
            }) {
                let _ = c.query_drop(format!("KILL QUERY {id}"));
            }
        });
    }
    fn close(&self) {
        let _ = self.cmd_tx.send(Cmd::Close);
        self.alive.store(false, Ordering::Relaxed);
    }
    fn select_all_sql(&self, table: &str) -> String {
        format!("SELECT * FROM {} LIMIT 100;", quote_ident(table))
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
        assert_eq!(quote_ident("a`b"), "`a``b`");
    }

    #[test]
    fn values_render_as_text() {
        assert_eq!(cell(&Value::NULL), "NULL");
        assert_eq!(cell(&Value::Bytes(b"hola".to_vec())), "hola");
        assert_eq!(cell(&Value::Bytes(vec![0xff, 0xfe])), "<blob 2 bytes>");
        assert_eq!(cell(&Value::Date(2024, 3, 5, 0, 0, 0, 0)), "2024-03-05");
        assert_eq!(cell(&Value::Date(2024, 3, 5, 7, 8, 9, 0)), "2024-03-05 07:08:09");
        assert_eq!(cell(&Value::Time(true, 1, 2, 3, 4, 0)), "-26:03:04");
    }

    #[test]
    fn ssl_mode_parsing_defaults_to_prefer() {
        assert_eq!(Ssl::parse("disable"), Ssl::Disable);
        assert_eq!(Ssl::parse("require"), Ssl::Require);
        assert_eq!(Ssl::parse("otra-cosa"), Ssl::Prefer);
    }

    // ---- against a live server: set RUSIO_MYSQL_TEST="host:port:user:password:database"
    fn live() -> Option<ConnectOptions> {
        let v = std::env::var("RUSIO_MYSQL_TEST").ok()?;
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
        let Some(mut opts) = live() else { return };
        opts.database = String::new();
        let mut c = connect(&opts).unwrap();
        c.query_drop("DROP DATABASE IF EXISTS rusio_t").unwrap();
        c.query_drop("CREATE DATABASE rusio_t").unwrap();
        c.query_drop("USE rusio_t").unwrap();
        c.query_drop(
            "CREATE TABLE persona(id INT AUTO_INCREMENT PRIMARY KEY, nombre VARCHAR(40), edad DECIMAL(5,1), nac DATE, foto BLOB)",
        )
        .unwrap();
        c.query_drop("INSERT INTO persona(nombre, edad, nac, foto) VALUES ('Ana', 30.5, '1990-01-02', x'FF00'), ('Luis', NULL, NULL, NULL)")
            .unwrap();
        c.query_drop("CREATE VIEW v AS SELECT nombre FROM persona").unwrap();

        let r = run_sql(&mut c, "SELECT * FROM persona ORDER BY id", 100).unwrap();
        let t = r.table.unwrap();
        assert_eq!(t.columns, ["id", "nombre", "edad", "nac", "foto"]);
        assert_eq!(t.rows[0], ["1", "Ana", "30.5", "1990-01-02", "<blob 2 bytes>"]);
        assert_eq!(t.rows[1], ["2", "Luis", "NULL", "NULL", "NULL"]);
        assert_eq!(r.message, "2 filas");

        let r = run_sql(&mut c, "SELECT * FROM persona", 1).unwrap();
        assert!(r.truncated);
        assert_eq!(r.table.unwrap().rows.len(), 1);

        let r = run_sql(&mut c, "SELECT nombre FROM persona WHERE 1=0", 10).unwrap();
        assert_eq!(r.table.unwrap().columns, ["nombre"]);

        let r = run_sql(
            &mut c,
            "UPDATE persona SET edad = 1; SELECT count(*) AS n FROM persona",
            10,
        )
        .unwrap();
        assert_eq!(r.table.unwrap().rows, [["2"]]);

        let r = run_sql(&mut c, "DELETE FROM persona", 10).unwrap();
        assert!(r.table.is_none());
        assert_eq!(r.message, "2 fila(s) afectada(s)");

        let e = run_sql(&mut c, "SELECT * FROM no_existe", 10).unwrap_err();
        assert!(e.contains("no_existe"), "{e}");
        assert!(run_sql(&mut c, "   ", 10).is_err());
        assert!(run_sql(&mut c, "SELECT 1", 10).is_ok());

        let schema = load_schema(&mut c).unwrap();
        let p = schema.iter().find(|t| t.name == "persona").unwrap();
        assert!(!p.is_view);
        assert_eq!(p.columns[1].0, "nombre");
        assert!(p.columns[1].1.starts_with("varchar"));
        assert!(schema.iter().find(|t| t.name == "v").unwrap().is_view);

        c.query_drop("DROP DATABASE rusio_t").unwrap();
    }

    #[test]
    fn live_empty_selection_lists_databases_and_read_only_blocks_writes() {
        let Some(mut opts) = live() else { return };
        opts.database = String::new();
        let mut c = connect(&opts).unwrap();
        assert!(load_schema(&mut c).unwrap().is_empty());
        let hint = databases_hint(&mut c, "", true).unwrap();
        assert!(hint.message.contains("No hay una base"));
        assert!(!hint.table.unwrap().rows.is_empty());

        let mut c = connect(&opts).unwrap();
        c.query_drop("DROP DATABASE IF EXISTS rusio_ro").unwrap();
        c.query_drop("CREATE DATABASE rusio_ro").unwrap();
        c.query_drop("CREATE TABLE rusio_ro.t(x INT)").unwrap();

        opts.read_only = true;
        let mut ro = connect(&opts).unwrap();
        let e = run_sql(&mut ro, "INSERT INTO rusio_ro.t VALUES (1)", 10).unwrap_err();
        assert!(e.to_lowercase().contains("read"), "{e}");
        assert!(run_sql(&mut ro, "SELECT * FROM rusio_ro.t", 10).is_ok());
        c.query_drop("DROP DATABASE rusio_ro").unwrap();
    }

    #[test]
    fn live_worker_switches_database_runs_and_cancels() {
        let Some(mut opts) = live() else { return };
        opts.database = String::new();
        let mut c = connect(&opts).unwrap();
        c.query_drop("DROP DATABASE IF EXISTS rusio_t2").unwrap();
        c.query_drop("CREATE DATABASE rusio_t2").unwrap();
        c.query_drop("CREATE TABLE rusio_t2.solo_en_t2(a INT)").unwrap();
        drop(c);

        let engine = Engine::start(connect(&opts).unwrap(), opts.clone(), true, 100, || {});
        let state = engine.state();
        assert!(engine.databases().contains(&"rusio_t2".to_string()));

        engine.switch_database("rusio_t2");
        wait_for(&state, "switch", |s| {
            !s.busy && s.tables.iter().any(|t| t.name == "solo_en_t2")
        });
        assert_eq!(engine.current_database(), "rusio_t2");

        engine.run("SELECT DATABASE() AS d".into());
        wait_for(&state, "query", |s| {
            !s.busy
                && s.result
                    .as_ref()
                    .and_then(|r| r.table.as_ref())
                    .is_some_and(|t| t.rows[0][0] == "rusio_t2")
        });

        engine.run("SELECT SLEEP(30)".into());
        wait_for(&state, "start", |s| s.busy);
        thread::sleep(Duration::from_millis(300));
        engine.cancel();
        wait_for(&state, "cancel", |s| !s.busy);
        // MySQL answers an interrupted SLEEP with a result; MariaDB with error 1317.
        engine.run("SELECT 7 AS z".into());
        wait_for(&state, "next", |s| {
            !s.busy
                && s.error.is_none()
                && s.result
                    .as_ref()
                    .and_then(|r| r.table.as_ref())
                    .is_some_and(|t| t.rows[0][0] == "7")
        });

        engine.switch_database("no_existe_xyz");
        wait_for(&state, "failed switch", |s| !s.busy && s.error.is_some());
        assert_eq!(engine.current_database(), "rusio_t2");
        engine.close();
        thread::sleep(Duration::from_millis(200));
        let mut c = connect(&opts).unwrap();
        c.query_drop("DROP DATABASE rusio_t2").unwrap();
    }
}
