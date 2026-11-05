//! SQLite access: opening the file, running statements and reading the schema. Pure
//! functions over a `Connection` (easy to test) plus a worker thread that owns the
//! connection so the UI never blocks on a query.
use crossbeam_channel::{unbounded, Receiver, Sender};
use fallible_iterator::FallibleIterator;
use parking_lot::Mutex;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, InterruptHandle, OpenFlags};
use rusio_plugin_api::data::DataTable;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

pub use rusio_plugin_api::sql::ConsoleState as EngineState;
use rusio_plugin_api::sql::{QueryResult, SchemaTable, SqlBackend};

pub enum Cmd {
    Run(String),
    RefreshSchema,
    Close,
}

pub struct OpenOptions {
    pub read_only: bool,
    pub create: bool,
    pub busy_timeout_ms: u64,
}

pub fn open(path: &Path, opts: &OpenOptions) -> Result<Connection, String> {
    if path.as_os_str().is_empty() {
        return Err("Indica la ruta del archivo de base de datos.".to_string());
    }
    if !opts.create && !path.exists() {
        return Err(format!("No existe el archivo: {}", path.display()));
    }
    let flags = if opts.read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else if opts.create {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } | OpenFlags::SQLITE_OPEN_URI;

    let conn = Connection::open_with_flags(path, flags).map_err(|e| e.to_string())?;
    conn.busy_timeout(Duration::from_millis(opts.busy_timeout_ms))
        .map_err(|e| e.to_string())?;
    // Opening is lazy: force a read so a non-database file fails here, not on the first query.
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))
        .map_err(|e| format!("No es una base de datos SQLite válida: {e}"))?;
    Ok(conn)
}

fn cell(v: ValueRef<'_>) -> String {
    match v {
        ValueRef::Null => "NULL".to_string(),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => f.to_string(),
        ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
        ValueRef::Blob(b) => format!("<blob {} bytes>", b.len()),
    }
}

fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Runs every statement in `sql`; the result is that of the last one.
pub fn run_sql(conn: &Connection, sql: &str, max_rows: usize) -> Result<QueryResult, String> {
    if sql.trim().is_empty() {
        return Err("Escribe una consulta.".to_string());
    }
    let start = Instant::now();
    let mut batch = rusqlite::Batch::new(conn, sql);
    let mut last: Option<(DataTable, bool)> = None;
    let mut changed = 0usize;
    let mut statements = 0usize;

    while let Some(mut stmt) = batch.next().map_err(|e| e.to_string())? {
        statements += 1;
        let ncols = stmt.column_count();
        if ncols > 0 {
            let columns: Vec<String> = stmt.column_names().into_iter().map(String::from).collect();
            let mut rows_it = stmt.query([]).map_err(|e| e.to_string())?;
            let mut rows = Vec::new();
            let mut truncated = false;
            while let Some(row) = rows_it.next().map_err(|e| e.to_string())? {
                if rows.len() >= max_rows {
                    truncated = true;
                    break;
                }
                rows.push(
                    (0..ncols)
                        .map(|i| row.get_ref(i).map(cell).unwrap_or_default())
                        .collect(),
                );
            }
            last = Some((DataTable { columns, rows }, truncated));
        } else {
            changed += stmt.execute([]).map_err(|e| e.to_string())?;
            last = None;
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
            QueryResult { table: Some(table), truncated, elapsed_ms, message }
        }
        None => QueryResult {
            table: None,
            truncated: false,
            elapsed_ms,
            message: format!("{changed} fila(s) afectada(s)"),
        },
    })
}

pub fn load_schema(conn: &Connection) -> Result<Vec<SchemaTable>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT name, type FROM sqlite_master \
             WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .map_err(|e| e.to_string())?;
    let items: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;

    let mut tables = Vec::with_capacity(items.len());
    for (name, kind) in items {
        let mut cols = Vec::new();
        let mut info = conn
            .prepare(&format!("PRAGMA table_info({})", quote_ident(&name)))
            .map_err(|e| e.to_string())?;
        let rows = info
            .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
            .map_err(|e| e.to_string())?;
        for c in rows {
            cols.push(c.map_err(|e| e.to_string())?);
        }
        tables.push(SchemaTable { name, is_view: kind == "view", columns: cols });
    }
    Ok(tables)
}

pub fn select_all_sql(table: &str) -> String {
    format!("SELECT * FROM {} LIMIT 100;", quote_ident(table))
}

/// Handle the UI keeps: the worker owns the connection.
pub struct Engine {
    cmd_tx: Sender<Cmd>,
    pub state: Arc<Mutex<EngineState>>,
    pub is_alive: Arc<AtomicBool>,
    interrupt: InterruptHandle,
}

impl Engine {
    pub fn start(
        conn: Connection,
        max_rows: usize,
        repaint: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (cmd_tx, cmd_rx): (Sender<Cmd>, Receiver<Cmd>) = unbounded();
        let state = Arc::new(Mutex::new(EngineState::default()));
        let is_alive = Arc::new(AtomicBool::new(true));
        let interrupt = conn.get_interrupt_handle();

        {
            let mut s = state.lock();
            match load_schema(&conn) {
                Ok(t) => s.tables = t,
                Err(e) => s.error = Some(e),
            }
        }

        let st = state.clone();
        let alive = is_alive.clone();
        thread::Builder::new()
            .name("sqlite-worker".to_string())
            .spawn(move || {
                while let Ok(cmd) = cmd_rx.recv() {
                    match cmd {
                        Cmd::Close => break,
                        Cmd::RefreshSchema => {
                            let schema = load_schema(&conn);
                            let mut s = st.lock();
                            match schema {
                                Ok(t) => s.tables = t,
                                Err(e) => s.error = Some(e),
                            }
                        }
                        Cmd::Run(sql) => {
                            st.lock().busy = true;
                            repaint();
                            let outcome = run_sql(&conn, &sql, max_rows);
                            // DDL/DML may have changed the schema.
                            let changed_schema = matches!(&outcome, Ok(r) if r.table.is_none());
                            let schema = changed_schema.then(|| load_schema(&conn));
                            let mut s = st.lock();
                            s.busy = false;
                            match outcome {
                                Ok(r) => {
                                    s.result = Some(r);
                                    s.error = None;
                                }
                                Err(e) => {
                                    s.error = Some(if e.contains("interrupted") {
                                        "Consulta cancelada.".to_string()
                                    } else {
                                        e
                                    });
                                }
                            }
                            if let Some(Ok(t)) = schema {
                                s.tables = t;
                            }
                        }
                    }
                    repaint();
                }
                alive.store(false, Ordering::Relaxed);
                repaint();
            })
            .expect("spawn sqlite worker");

        Self { cmd_tx, state, is_alive, interrupt }
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn cancel(&self) {
        self.interrupt.interrupt();
    }

    pub fn close(&self) {
        self.interrupt.interrupt();
        let _ = self.cmd_tx.send(Cmd::Close);
        self.is_alive.store(false, Ordering::Relaxed);
    }
}

impl SqlBackend for Engine {
    fn state(&self) -> Arc<Mutex<EngineState>> {
        self.state.clone()
    }
    fn is_alive(&self) -> bool {
        self.is_alive.load(Ordering::Relaxed)
    }
    fn run(&self, sql: String) {
        self.send(Cmd::Run(sql));
    }
    fn refresh_schema(&self) {
        self.send(Cmd::RefreshSchema);
    }
    fn cancel(&self) {
        Engine::cancel(self);
    }
    fn close(&self) {
        Engine::close(self);
    }
    fn select_all_sql(&self, table: &str) -> String {
        select_all_sql(table)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE persona(id INTEGER PRIMARY KEY, nombre TEXT, foto BLOB, edad REAL);
             INSERT INTO persona(nombre, foto, edad) VALUES ('Ana', x'0102', 30.5), ('Luis', NULL, NULL);
             CREATE VIEW v AS SELECT nombre FROM persona;",
        )
        .unwrap();
        c
    }

    #[test]
    fn select_renders_every_value_kind() {
        let r = run_sql(&mem(), "SELECT * FROM persona ORDER BY id", 100).unwrap();
        let t = r.table.unwrap();
        assert_eq!(t.columns, ["id", "nombre", "foto", "edad"]);
        assert_eq!(t.rows[0], ["1", "Ana", "<blob 2 bytes>", "30.5"]);
        assert_eq!(t.rows[1], ["2", "Luis", "NULL", "NULL"]);
        assert_eq!(r.message, "2 filas");
        assert!(!r.truncated);
    }

    #[test]
    fn result_is_limited_to_max_rows() {
        let r = run_sql(&mem(), "SELECT * FROM persona", 1).unwrap();
        assert!(r.truncated);
        assert_eq!(r.table.unwrap().rows.len(), 1);
    }

    #[test]
    fn last_statement_wins_and_dml_reports_changes() {
        let c = mem();
        let r = run_sql(&c, "UPDATE persona SET edad = 1; SELECT count(*) AS n FROM persona", 10).unwrap();
        assert_eq!(r.table.unwrap().rows, [["2"]]);

        let r = run_sql(&c, "DELETE FROM persona", 10).unwrap();
        assert!(r.table.is_none());
        assert_eq!(r.message, "2 fila(s) afectada(s)");
    }

    #[test]
    fn errors_are_reported_not_panicked() {
        assert!(run_sql(&mem(), "SELEC 1", 10).is_err());
        assert!(run_sql(&mem(), "   ", 10).is_err());
        assert!(run_sql(&mem(), "SELECT * FROM no_existe", 10).unwrap_err().contains("no_existe"));
    }

    #[test]
    fn schema_lists_tables_views_and_columns() {
        let s = load_schema(&mem()).unwrap();
        let p = s.iter().find(|t| t.name == "persona").unwrap();
        assert!(!p.is_view);
        assert_eq!(p.columns[1], ("nombre".to_string(), "TEXT".to_string()));
        assert!(s.iter().find(|t| t.name == "v").unwrap().is_view);
    }

    #[test]
    fn identifiers_are_quoted() {
        assert_eq!(select_all_sql("a\"b"), "SELECT * FROM \"a\"\"b\" LIMIT 100;");
    }

    #[test]
    fn open_rejects_missing_and_non_database_files() {
        let opts = OpenOptions { read_only: false, create: false, busy_timeout_ms: 100 };
        assert!(open(Path::new("no-existe-12345.db"), &opts).unwrap_err().contains("No existe"));

        let dir = std::env::temp_dir().join(format!("rusio-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let junk = dir.join("junk.db");
        std::fs::write(&junk, b"esto no es sqlite, solo texto de relleno largo largo largo largo").unwrap();
        assert!(open(&junk, &opts).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_only_blocks_writes() {
        let dir = std::env::temp_dir().join(format!("rusio-ro-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.db");
        let create = OpenOptions { read_only: false, create: true, busy_timeout_ms: 100 };
        open(&path, &create)
            .unwrap()
            .execute_batch("CREATE TABLE t(x)")
            .unwrap();

        let ro = OpenOptions { read_only: true, create: false, busy_timeout_ms: 100 };
        let c = open(&path, &ro).unwrap();
        assert!(run_sql(&c, "INSERT INTO t VALUES (1)", 10).is_err());
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn wait_for(engine: &Engine, what: &str, mut cond: impl FnMut(&EngineState) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if cond(&engine.state.lock()) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for {what}");
    }

    #[test]
    fn worker_runs_queries_and_refreshes_schema_after_ddl() {
        let engine = Engine::start(mem(), 100, || {});
        engine.send(Cmd::Run("SELECT nombre FROM persona ORDER BY id".into()));
        wait_for(&engine, "select", |s| s.result.is_some() && !s.busy);
        assert_eq!(
            engine.state.lock().result.as_ref().unwrap().table.as_ref().unwrap().rows[0][0],
            "Ana"
        );

        engine.send(Cmd::Run("CREATE TABLE nueva(a)".into()));
        wait_for(&engine, "schema refresh", |s| s.tables.iter().any(|t| t.name == "nueva"));
        engine.close();
    }

    #[test]
    fn long_query_can_be_cancelled() {
        let engine = Engine::start(mem(), 100, || {});
        // Counts forever until interrupted.
        engine.send(Cmd::Run(
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c) SELECT count(*) FROM c".into(),
        ));
        wait_for(&engine, "query to start", |s| s.busy);
        engine.cancel();
        wait_for(&engine, "cancellation", |s| !s.busy && s.error.is_some());
        assert_eq!(engine.state.lock().error.as_deref(), Some("Consulta cancelada."));

        // The engine keeps working after a cancel.
        engine.send(Cmd::Run("SELECT 1".into()));
        wait_for(&engine, "next query", |s| s.error.is_none() && s.result.is_some() && !s.busy);
        engine.close();
    }
}
