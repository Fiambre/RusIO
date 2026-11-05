//! MongoDB access: connecting, running shell-style commands and reading the "schema"
//! (collections with the fields of one sample document), plus a worker thread that owns the
//! client so the UI never blocks on a query.
use crossbeam_channel::{unbounded, Receiver, Sender};
use mongodb::bson::{doc, Bson, Document};
use mongodb::options::{ClientOptions, Credential, ServerAddress, Tls, TlsOptions};
use mongodb::results::CollectionType;
use mongodb::sync::Client;
use parking_lot::Mutex;
use rusio_plugin_api::data::DataTable;
use rusio_plugin_api::sql::{ConsoleState, QueryResult, SchemaTable, SqlBackend};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::shell::{self, Op};

/// How many collections get a sample document for the field list (one query each).
const SAMPLE_LIMIT: usize = 100;
/// Columns shown for a result; documents can have thousands of distinct keys.
const MAX_COLUMNS: usize = 200;

#[derive(Debug, Clone)]
pub struct ConnectOptions {
    /// `mongodb://...`; when set it replaces host, port and credentials.
    pub uri: Option<String>,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Option<String>,
    pub auth_source: String,
    /// Empty = none selected.
    pub database: String,
    pub tls: bool,
    pub read_only: bool,
    pub connect_timeout_secs: u64,
    /// 0 = no limit.
    pub query_timeout_secs: u64,
}

pub fn connect(o: &ConnectOptions) -> Result<Client, String> {
    let timeout = Duration::from_secs(o.connect_timeout_secs.max(1));
    let mut options = match &o.uri {
        Some(uri) => ClientOptions::parse(uri.as_str()).run().map_err(describe)?,
        None => {
            if o.host.trim().is_empty() {
                return Err("Indica el servidor.".to_string());
            }
            let mut b = ClientOptions::default();
            b.hosts = vec![ServerAddress::Tcp {
                host: o.host.clone(),
                port: Some(o.port),
            }];
            // One server, even if it belongs to a replica set (the address may be a tunnel).
            b.direct_connection = Some(true);
            if !o.user.is_empty() {
                let mut c = Credential::default();
                c.username = Some(o.user.clone());
                c.password = o.password.clone();
                c.source = Some(if o.auth_source.is_empty() {
                    "admin".to_string()
                } else {
                    o.auth_source.clone()
                });
                b.credential = Some(c);
            }
            if o.tls {
                let mut t = TlsOptions::default();
                // Like the other database plugins: encrypted, certificate not verified.
                t.allow_invalid_certificates = Some(true);
                b.tls = Some(Tls::Enabled(t));
            }
            b
        }
    };
    options.connect_timeout = Some(timeout);
    options.server_selection_timeout = Some(timeout);
    options.app_name = Some("RusIO".to_string());

    let client = Client::with_options(options).map_err(describe)?;
    // The driver connects lazily: force a round trip so bad addresses and credentials fail here.
    client
        .database("admin")
        .run_command(doc! { "ping": 1 })
        .run()
        .map_err(describe)?;
    Ok(client)
}

fn describe(e: mongodb::error::Error) -> String {
    e.to_string()
}

// ---------------------------------------------------------------- values -> table

fn cell(b: &Bson) -> String {
    match b {
        Bson::Null => "NULL".to_string(),
        Bson::String(s) => s.clone(),
        Bson::ObjectId(id) => id.to_hex(),
        Bson::Int32(n) => n.to_string(),
        Bson::Int64(n) => n.to_string(),
        Bson::Double(f) => f.to_string(),
        Bson::Boolean(v) => v.to_string(),
        Bson::DateTime(d) => d.try_to_rfc3339_string().unwrap_or_else(|_| d.to_string()),
        Bson::Binary(bin) => format!("<binario {} bytes>", bin.bytes.len()),
        Bson::Document(_) | Bson::Array(_) => {
            serde_json::to_string(&b.clone().into_relaxed_extjson()).unwrap_or_else(|_| b.to_string())
        }
        other => other.to_string(),
    }
}

/// One column per top-level key (first-seen order, `_id` first); nested values as JSON text.
pub fn docs_to_table(docs: &[Document]) -> DataTable {
    let mut columns: Vec<String> = Vec::new();
    for d in docs {
        for k in d.keys() {
            if columns.len() < MAX_COLUMNS && !columns.iter().any(|c| c == k) {
                columns.push(k.clone());
            }
        }
    }
    if let Some(pos) = columns.iter().position(|c| c == "_id") {
        let id = columns.remove(pos);
        columns.insert(0, id);
    }
    let rows = docs
        .iter()
        .map(|d| {
            columns
                .iter()
                .map(|c| d.get(c).map_or_else(String::new, cell))
                .collect()
        })
        .collect();
    DataTable { columns, rows }
}

fn type_name(b: &Bson) -> String {
    format!("{:?}", b.element_type()).to_lowercase()
}

// ---------------------------------------------------------------- running

struct Limits {
    max_rows: usize,
    max_time: Option<Duration>,
    read_only: bool,
}

fn table_result(docs: Vec<Document>, truncated: bool, start: Instant) -> QueryResult {
    let n = docs.len();
    let message = if truncated {
        format!("{n} documentos (resultado limitado)")
    } else if n == 1 {
        "1 documento".to_string()
    } else {
        format!("{n} documentos")
    };
    QueryResult {
        table: Some(docs_to_table(&docs)),
        truncated,
        elapsed_ms: start.elapsed().as_millis(),
        message,
    }
}

fn message_result(message: String, start: Instant) -> QueryResult {
    QueryResult {
        table: None,
        truncated: false,
        elapsed_ms: start.elapsed().as_millis(),
        message,
    }
}

/// Reads up to `max_rows` documents; the flag says there were more.
fn take_docs(
    cursor: impl Iterator<Item = mongodb::error::Result<Document>>,
    max_rows: usize,
) -> Result<(Vec<Document>, bool), String> {
    let mut docs = Vec::new();
    for d in cursor {
        if docs.len() >= max_rows {
            return Ok((docs, true));
        }
        docs.push(d.map_err(describe)?);
    }
    Ok((docs, false))
}

fn run(client: &Client, db: &str, text: &str, lim: &Limits) -> Result<QueryResult, String> {
    let op = shell::parse(text)?;
    if lim.read_only && op.is_write() {
        // Commands could be reads too, but we cannot tell: be strict.
        return Err("La conexión es de solo lectura: no se permiten escrituras ni comandos.".to_string());
    }
    let needs_db = !matches!(op, Op::Command(_));
    if needs_db && db.is_empty() {
        return Err("Elige una base de datos en el selector de la izquierda.".to_string());
    }
    let database = client.database(if db.is_empty() { "admin" } else { db });
    let start = Instant::now();

    match op {
        Op::Find {
            coll,
            filter,
            projection,
            sort,
            limit,
            skip,
        } => {
            let c = database.collection::<Document>(&coll);
            // One more than we show tells us whether the result was cut.
            let cap = limit
                .filter(|l| *l > 0)
                .map_or(lim.max_rows, |l| (l as usize).min(lim.max_rows));
            let mut find = c.find(filter).limit(cap as i64 + 1);
            if let Some(p) = projection {
                find = find.projection(p);
            }
            if let Some(s) = sort {
                find = find.sort(s);
            }
            if let Some(n) = skip {
                find = find.skip(n);
            }
            if let Some(t) = lim.max_time {
                find = find.max_time(t);
            }
            let (docs, more) = take_docs(find.run().map_err(describe)?, cap)?;
            // Hitting the user's own limit is not truncation; hitting ours is.
            let truncated = more && limit.filter(|l| *l > 0).is_none_or(|l| l as usize > lim.max_rows);
            Ok(table_result(docs, truncated, start))
        }
        Op::Aggregate { coll, pipeline } => {
            let c = database.collection::<Document>(&coll);
            let mut agg = c.aggregate(pipeline);
            if let Some(t) = lim.max_time {
                agg = agg.max_time(t);
            }
            let (docs, truncated) = take_docs(agg.run().map_err(describe)?, lim.max_rows)?;
            Ok(table_result(docs, truncated, start))
        }
        Op::Count { coll, filter } => {
            let c = database.collection::<Document>(&coll);
            let n = match filter {
                Some(f) => c.count_documents(f).run().map_err(describe)?,
                None => c.estimated_document_count().run().map_err(describe)?,
            };
            Ok(table_result(vec![doc! { "count": n as i64 }], false, start))
        }
        Op::Distinct { coll, field, filter } => {
            let c = database.collection::<Document>(&coll);
            let mut d = c.distinct(field.as_str(), filter.unwrap_or_default());
            if let Some(t) = lim.max_time {
                d = d.max_time(t);
            }
            let values = d.run().map_err(describe)?;
            let truncated = values.len() > lim.max_rows;
            let docs = values
                .into_iter()
                .take(lim.max_rows)
                .map(|v| doc! { field.as_str(): v })
                .collect();
            Ok(table_result(docs, truncated, start))
        }
        Op::InsertOne { coll, doc: d } => {
            let r = database
                .collection::<Document>(&coll)
                .insert_one(d)
                .run()
                .map_err(describe)?;
            Ok(message_result(
                format!("1 documento insertado (_id: {})", cell(&r.inserted_id)),
                start,
            ))
        }
        Op::InsertMany { coll, docs } => {
            let r = database
                .collection::<Document>(&coll)
                .insert_many(docs)
                .run()
                .map_err(describe)?;
            Ok(message_result(
                format!("{} documento(s) insertado(s)", r.inserted_ids.len()),
                start,
            ))
        }
        Op::Update {
            coll,
            filter,
            update,
            many,
        } => {
            let c = database.collection::<Document>(&coll);
            let r = if many {
                c.update_many(filter, update).run()
            } else {
                c.update_one(filter, update).run()
            }
            .map_err(describe)?;
            Ok(message_result(
                format!("{} coincidente(s), {} modificado(s)", r.matched_count, r.modified_count),
                start,
            ))
        }
        Op::Delete { coll, filter, many } => {
            let c = database.collection::<Document>(&coll);
            let r = if many {
                c.delete_many(filter).run()
            } else {
                c.delete_one(filter).run()
            }
            .map_err(describe)?;
            Ok(message_result(
                format!("{} documento(s) eliminado(s)", r.deleted_count),
                start,
            ))
        }
        Op::Command(cmd) => {
            let reply = database.run_command(cmd).run().map_err(describe)?;
            Ok(table_result(vec![reply], false, start))
        }
    }
}

// ---------------------------------------------------------------- schema

pub fn list_databases(client: &Client) -> Vec<String> {
    client.list_database_names().run().unwrap_or_default()
}

/// Collections of `db` with the top-level fields of one sample document each.
pub fn load_schema(client: &Client, db: &str) -> Result<Vec<SchemaTable>, String> {
    if db.is_empty() {
        return Ok(Vec::new());
    }
    let database = client.database(db);
    let mut specs: Vec<(String, bool)> = Vec::new();
    for spec in database.list_collections().run().map_err(describe)? {
        let spec = spec.map_err(describe)?;
        specs.push((spec.name, spec.collection_type == CollectionType::View));
    }
    specs.sort();
    Ok(specs
        .into_iter()
        .enumerate()
        .map(|(i, (name, is_view))| {
            let columns = if i < SAMPLE_LIMIT {
                database
                    .collection::<Document>(&name)
                    .find_one(doc! {})
                    .run()
                    .ok()
                    .flatten()
                    .map(|d| d.iter().map(|(k, v)| (k.clone(), type_name(v))).collect())
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            SchemaTable { name, is_view, columns }
        })
        .collect())
}

pub enum Cmd {
    Run(String),
    RefreshSchema,
    Switch(String),
    Close,
}

fn apply_schema(schema: Result<Vec<SchemaTable>, String>, state: &Mutex<ConsoleState>) {
    let mut s = state.lock();
    match schema {
        Ok(t) => s.tables = t,
        Err(e) => s.error = Some(e),
    }
}

/// Handle the UI keeps: the worker owns the client.
pub struct Engine {
    cmd_tx: Sender<Cmd>,
    state: Arc<Mutex<ConsoleState>>,
    alive: Arc<AtomicBool>,
    dbs: Arc<Mutex<Vec<String>>>,
    current: Arc<Mutex<String>>,
}

impl Engine {
    pub fn start(
        client: Client,
        opts: ConnectOptions,
        max_rows: usize,
        repaint: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (cmd_tx, cmd_rx): (Sender<Cmd>, Receiver<Cmd>) = unbounded();
        let state = Arc::new(Mutex::new(ConsoleState::default()));
        let alive = Arc::new(AtomicBool::new(true));
        let dbs = Arc::new(Mutex::new(list_databases(&client)));
        let current = Arc::new(Mutex::new(opts.database.clone()));
        let limits = Limits {
            max_rows,
            max_time: (opts.query_timeout_secs > 0).then(|| Duration::from_secs(opts.query_timeout_secs)),
            read_only: opts.read_only,
        };

        apply_schema(load_schema(&client, &opts.database), &state);
        if opts.database.is_empty() {
            state.lock().result = Some(message_result(
                "No hay una base de datos seleccionada: elígela en el selector de la izquierda.".to_string(),
                Instant::now(),
            ));
        }

        let (st, al, dbs_w, cur_w) = (state.clone(), alive.clone(), dbs.clone(), current.clone());
        thread::Builder::new()
            .name("mongodb-worker".to_string())
            .spawn(move || {
                while let Ok(cmd) = cmd_rx.recv() {
                    match cmd {
                        Cmd::Close => break,
                        Cmd::RefreshSchema => {
                            let db = cur_w.lock().clone();
                            apply_schema(load_schema(&client, &db), &st);
                            *dbs_w.lock() = list_databases(&client);
                        }
                        Cmd::Switch(db) => {
                            st.lock().busy = true;
                            repaint();
                            let schema = load_schema(&client, &db);
                            let mut s = st.lock();
                            s.busy = false;
                            match schema {
                                Ok(t) => {
                                    *cur_w.lock() = db;
                                    s.tables = t;
                                    s.result = None;
                                    s.error = None;
                                }
                                Err(e) => s.error = Some(e),
                            }
                        }
                        Cmd::Run(text) => {
                            st.lock().busy = true;
                            repaint();
                            let db = cur_w.lock().clone();
                            let outcome = run(&client, &db, &text, &limits);
                            // Writes and commands may have created or dropped collections.
                            if matches!(&outcome, Ok(r) if r.table.is_none()) {
                                apply_schema(load_schema(&client, &db), &st);
                            }
                            let mut s = st.lock();
                            s.busy = false;
                            match outcome {
                                Ok(r) => {
                                    s.result = Some(r);
                                    s.error = None;
                                }
                                Err(e) => s.error = Some(e),
                            }
                        }
                    }
                    repaint();
                }
                al.store(false, Ordering::Relaxed);
                repaint();
            })
            .expect("spawn mongodb worker");

        Self {
            cmd_tx,
            state,
            alive,
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
        // The driver has no way to interrupt a running operation from the client.
        self.state.lock().error = Some(
            "MongoDB no permite cancelar una consulta en curso: usa el «Tiempo máximo por consulta» de los ajustes."
                .to_string(),
        );
    }
    fn close(&self) {
        let _ = self.cmd_tx.send(Cmd::Close);
        self.alive.store(false, Ordering::Relaxed);
    }
    fn select_all_sql(&self, table: &str) -> String {
        format!(
            "db.getCollection({}).find({{}}).limit(100)",
            serde_json::to_string(table).unwrap_or_default()
        )
    }
    fn is_sql(&self) -> bool {
        false
    }
    fn completion_words(&self) -> &'static [&'static str] {
        crate::shell::WORDS
    }
    fn destructive(&self, text: &str) -> Vec<String> {
        // A parse error is reported when the query runs, not here.
        shell::parse(text).map(|op| shell::destructive(&op)).unwrap_or_default()
    }
    fn editor_hint(&self) -> &'static str {
        "db.coleccion.find({campo: valor})   (Ctrl+Enter para ejecutar)"
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
    use mongodb::bson::oid::ObjectId;

    #[test]
    fn cells_render_every_common_kind() {
        assert_eq!(cell(&Bson::Null), "NULL");
        assert_eq!(cell(&Bson::String("a".into())), "a");
        assert_eq!(cell(&Bson::Int32(3)), "3");
        assert_eq!(
            cell(&Bson::ObjectId(
                ObjectId::parse_str("650000000000000000000001").unwrap()
            )),
            "650000000000000000000001"
        );
        assert_eq!(cell(&Bson::Document(doc! { "a": 1 })), "{\"a\":1}");
        assert_eq!(cell(&Bson::Array(vec![Bson::Int32(1), Bson::Int32(2)])), "[1,2]");
    }

    #[test]
    fn table_has_the_union_of_keys_with_id_first() {
        let t = docs_to_table(&[doc! { "a": 1, "_id": 7 }, doc! { "b": "x", "a": 2 }]);
        assert_eq!(t.columns, ["_id", "a", "b"]);
        assert_eq!(t.rows[0], ["7", "1", ""]);
        assert_eq!(t.rows[1], ["", "2", "x"]);
    }

    // ---- against a live server: set RUSIO_MONGO_TEST="host:port"  (no authentication)
    fn live() -> Option<ConnectOptions> {
        let v = std::env::var("RUSIO_MONGO_TEST").ok()?;
        let (host, port) = v.split_once(':')?;
        Some(ConnectOptions {
            uri: None,
            host: host.to_string(),
            port: port.parse().ok()?,
            user: String::new(),
            password: None,
            auth_source: String::new(),
            database: "rusio_t".to_string(),
            tls: false,
            read_only: false,
            connect_timeout_secs: 5,
            query_timeout_secs: 0,
        })
    }

    fn lim(max_rows: usize, read_only: bool) -> Limits {
        Limits {
            max_rows,
            max_time: None,
            read_only,
        }
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
    fn live_crud_queries_and_errors() {
        let Some(opts) = live() else { return };
        let c = connect(&opts).unwrap();
        c.database("rusio_t").drop().run().unwrap();
        let l = lim(100, false);
        let go = |text: &str| run(&c, "rusio_t", text, &l);

        let r = go("db.persona.insertMany([{nombre: 'Ana', edad: 30, tags: ['a','b']}, {nombre: 'Luis', edad: 25}, {nombre: 'Eva', edad: 41}])").unwrap();
        assert_eq!(r.message, "3 documento(s) insertado(s)");

        let r = go("db.persona.find({edad: {$gt: 26}}).sort({edad: 1})").unwrap();
        let t = r.table.unwrap();
        assert_eq!(t.columns[0], "_id");
        let nombre = t.columns.iter().position(|c| c == "nombre").unwrap();
        assert_eq!(
            t.rows.iter().map(|r| r[nombre].as_str()).collect::<Vec<_>>(),
            ["Ana", "Eva"]
        );
        assert_eq!(r.message, "2 documentos");

        // Arrays render as JSON; a field missing from some documents is blank.
        let r = go("db.persona.find({nombre: 'Ana'})").unwrap();
        let t = r.table.unwrap();
        let tags = t.columns.iter().position(|c| c == "tags").unwrap();
        assert_eq!(t.rows[0][tags], "[\"a\",\"b\"]");

        // The user's own limit is not "truncated"; ours is.
        assert!(!go("db.persona.find().limit(2)").unwrap().truncated);
        let small = lim(2, false);
        let r = run(&c, "rusio_t", "db.persona.find()", &small).unwrap();
        assert!(r.truncated);
        assert_eq!(r.table.unwrap().rows.len(), 2);

        let r = go("db.persona.aggregate([{$group: {_id: null, total: {$sum: '$edad'}}}])").unwrap();
        assert_eq!(r.table.unwrap().rows[0][1], "96");
        assert_eq!(
            go("db.persona.countDocuments({edad: {$lt: 40}})")
                .unwrap()
                .table
                .unwrap()
                .rows[0][0],
            "2"
        );
        assert_eq!(
            go("db.persona.distinct('nombre')").unwrap().table.unwrap().rows.len(),
            3
        );

        // ObjectId round trip through the helper.
        let r = go("db.persona.findOne({nombre: 'Luis'})").unwrap();
        let t = r.table.unwrap();
        let id = t.rows[0][0].clone();
        let r = go(&format!("db.persona.find({{_id: ObjectId(\"{id}\")}})")).unwrap();
        assert_eq!(r.table.unwrap().rows.len(), 1);

        let r = go("db.persona.updateMany({edad: {$lt: 40}}, {$set: {joven: true}})").unwrap();
        assert_eq!(r.message, "2 coincidente(s), 2 modificado(s)");
        let r = go("db.persona.deleteOne({nombre: 'Eva'})").unwrap();
        assert_eq!(r.message, "1 documento(s) eliminado(s)");

        let r = go("{ping: 1}").unwrap();
        assert!(r.table.unwrap().columns.contains(&"ok".to_string()));

        assert!(go("db.persona.find({$bad: 1})").is_err());
        assert!(go("db.persona.find(").is_err());
        assert!(go("db.persona.find()").is_ok(), "connection survives errors");

        let schema = load_schema(&c, "rusio_t").unwrap();
        let p = schema.iter().find(|t| t.name == "persona").unwrap();
        assert!(p.columns.iter().any(|(k, ty)| k == "nombre" && ty == "string"));
        assert!(list_databases(&c).contains(&"rusio_t".to_string()));

        c.database("rusio_t").drop().run().unwrap();
    }

    #[test]
    fn live_read_only_blocks_writes_and_no_database_is_explained() {
        let Some(mut opts) = live() else { return };
        let c = connect(&opts).unwrap();
        let ro = lim(10, true);
        assert!(run(&c, "rusio_t", "db.x.insertOne({a: 1})", &ro)
            .unwrap_err()
            .contains("solo lectura"));
        assert!(run(&c, "rusio_t", "{drop: 'x'}", &ro)
            .unwrap_err()
            .contains("solo lectura"));
        assert!(run(&c, "rusio_t", "db.x.find()", &ro).is_ok());
        assert!(run(&c, "", "db.x.find()", &lim(10, false))
            .unwrap_err()
            .contains("selector"));

        opts.host = "127.0.0.1".to_string();
        opts.port = 1; // nothing listens here
        opts.connect_timeout_secs = 1;
        assert!(connect(&opts).is_err());
    }

    #[test]
    fn live_worker_switches_database() {
        let Some(mut opts) = live() else { return };
        let c = connect(&opts).unwrap();
        c.database("rusio_t3")
            .collection::<Document>("solo_aqui")
            .insert_one(doc! { "a": 1 })
            .run()
            .unwrap();
        opts.database = String::new();

        let engine = Engine::start(connect(&opts).unwrap(), opts.clone(), 100, || {});
        let state = engine.state();
        assert!(engine.databases().contains(&"rusio_t3".to_string()));
        assert!(state
            .lock()
            .result
            .as_ref()
            .unwrap()
            .message
            .contains("No hay una base"));

        engine.switch_database("rusio_t3");
        wait_for(&state, "switch", |s| {
            !s.busy && s.tables.iter().any(|t| t.name == "solo_aqui")
        });
        assert_eq!(engine.current_database(), "rusio_t3");

        engine.run("db.solo_aqui.find()".into());
        wait_for(&state, "query", |s| {
            !s.busy && s.result.as_ref().is_some_and(|r| r.message == "1 documento")
        });
        engine.close();
        c.database("rusio_t3").drop().run().unwrap();
    }

    // RUSIO_MONGO_AUTH="usuario:clave" for a server started with --auth (user in the admin db).
    #[test]
    fn live_authentication_accepts_the_password_and_rejects_a_wrong_one() {
        let (Some(mut opts), Ok(cred)) = (live(), std::env::var("RUSIO_MONGO_AUTH")) else {
            return;
        };
        let (user, pass) = cred.split_once(':').unwrap();
        opts.user = user.to_string();
        opts.password = Some(pass.to_string());
        let c = connect(&opts).unwrap();
        assert!(run(&c, "rusio_t", "db.x.insertOne({a: 1})", &lim(10, false)).is_ok());
        let _ = c.database("rusio_t").drop().run();

        opts.password = Some("clave-incorrecta".to_string());
        assert!(connect(&opts).is_err());
        opts.user = String::new();
        // Without credentials the ping works but real operations are refused.
        if let Ok(c) = connect(&opts) {
            assert!(run(&c, "rusio_t", "db.x.find()", &lim(10, false)).is_err());
        }
    }
}
