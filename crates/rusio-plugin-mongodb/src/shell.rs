//! A small subset of the mongo shell syntax: `db.coleccion.find({...}).sort({...}).limit(10)`.
//!
//! Arguments are relaxed JSON (unquoted keys, single quotes, trailing commas) and the usual
//! shell helpers `ObjectId("..")`, `ISODate("..")`, `NumberLong(..)` and `NumberInt(..)` work.
//! A bare `{ ... }` is sent as a database command (`{ "ping": 1 }`).
use mongodb::bson::{Bson, Document};

#[derive(Debug, PartialEq)]
pub enum Op {
    Find {
        coll: String,
        filter: Document,
        projection: Option<Document>,
        sort: Option<Document>,
        limit: Option<i64>,
        skip: Option<u64>,
    },
    Aggregate {
        coll: String,
        pipeline: Vec<Document>,
    },
    Count {
        coll: String,
        filter: Option<Document>,
    },
    Distinct {
        coll: String,
        field: String,
        filter: Option<Document>,
    },
    InsertOne {
        coll: String,
        doc: Document,
    },
    InsertMany {
        coll: String,
        docs: Vec<Document>,
    },
    Update {
        coll: String,
        filter: Document,
        update: Document,
        many: bool,
    },
    Delete {
        coll: String,
        filter: Document,
        many: bool,
    },
    Command(Document),
}

impl Op {
    pub fn is_write(&self) -> bool {
        matches!(
            self,
            Op::InsertOne { .. } | Op::InsertMany { .. } | Op::Update { .. } | Op::Delete { .. } | Op::Command(_)
        )
    }
}

/// Why running `op` could destroy data (empty = nothing risky).
pub fn destructive(op: &Op) -> Vec<String> {
    let mut r = Vec::new();
    match op {
        Op::Delete { filter, many, .. } => {
            if filter.is_empty() {
                r.push("borra TODOS los documentos de la colección (filtro vacío)".to_string());
            } else if *many {
                r.push("deleteMany borra todos los documentos que coincidan".to_string());
            }
        }
        Op::Update { filter, many: true, .. } if filter.is_empty() => {
            r.push("updateMany sin filtro modifica TODOS los documentos".to_string());
        }
        Op::Command(cmd) => {
            if let Some(first) = cmd.keys().next() {
                if matches!(
                    first.as_str(),
                    "drop" | "dropDatabase" | "dropIndexes" | "delete" | "renameCollection"
                ) {
                    r.push(format!("el comando «{first}» elimina o renombra datos"));
                }
            }
        }
        _ => {}
    }
    r
}

/// Names offered by the editor: shell methods, helpers and the most used operators.
pub const WORDS: &[&str] = &[
    "db",
    "find",
    "findOne",
    "aggregate",
    "countDocuments",
    "estimatedDocumentCount",
    "distinct",
    "insertOne",
    "insertMany",
    "updateOne",
    "updateMany",
    "deleteOne",
    "deleteMany",
    "runCommand",
    "getCollection",
    "sort",
    "limit",
    "skip",
    "ObjectId",
    "ISODate",
    "NumberLong",
    "NumberInt",
    "$set",
    "$unset",
    "$inc",
    "$push",
    "$pull",
    "$addToSet",
    "$match",
    "$group",
    "$project",
    "$sort",
    "$limit",
    "$skip",
    "$lookup",
    "$unwind",
    "$count",
    "$sum",
    "$avg",
    "$min",
    "$max",
    "$first",
    "$last",
    "$eq",
    "$ne",
    "$gt",
    "$gte",
    "$lt",
    "$lte",
    "$in",
    "$nin",
    "$and",
    "$or",
    "$not",
    "$exists",
    "$regex",
    "$expr",
    "true",
    "false",
    "null",
];

const SUPPORTED: &str = "find, findOne, aggregate, countDocuments, estimatedDocumentCount, distinct, insertOne, \
    insertMany, updateOne, updateMany, deleteOne, deleteMany";

/// Index of the bracket closing the one at `open`, skipping strings.
fn matching(s: &str, open: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0i32;
    let mut i = open;
    while i < b.len() {
        match b[i] {
            q @ (b'"' | b'\'' | b'`') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Splits `a, {b: 1}, [c, d]` on the commas that are not nested or quoted.
fn split_args(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let (mut out, mut start, mut depth, mut i) = (Vec::new(), 0, 0i32, 0);
    while i < b.len() {
        match b[i] {
            q @ (b'"' | b'\'' | b'`') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if !s[start..].trim().is_empty() {
        out.push(&s[start..]);
    }
    out
}

/// `ObjectId("x")` -> `{"$oid":"x"}` and friends, outside of strings.
fn expand_helpers(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'"' || c == b'\'' || c == b'`' {
            let start = i;
            i += 1;
            while i < b.len() && b[i] != c {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(b.len());
            out.push_str(&s[start..i]);
            continue;
        }
        if c.is_ascii_alphabetic() {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let word = &s[start..i];
            let helper = matches!(word, "ObjectId" | "ISODate" | "NumberLong" | "NumberInt");
            if helper && b.get(i) == Some(&b'(') {
                if let Some(close) = matching(s, i) {
                    let inner = expand_helpers(s[i + 1..close].trim());
                    match word {
                        "ObjectId" => out.push_str(&format!("{{\"$oid\":{inner}}}")),
                        "ISODate" => out.push_str(&format!("{{\"$date\":{inner}}}")),
                        // Numbers pass through; quoted numbers are unquoted.
                        _ => out.push_str(inner.trim_matches(|c| c == '"' || c == '\'')),
                    }
                    i = close + 1;
                    continue;
                }
            }
            out.push_str(word);
            continue;
        }
        // Copy one full character.
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn value(text: &str) -> Result<Bson, String> {
    let text = expand_helpers(text.trim());
    let v: serde_json::Value =
        json5::from_str(&text).map_err(|e| format!("JSON inválido en «{}»: {e}", text.trim()))?;
    Bson::try_from(v).map_err(|e| format!("valor no válido: {e}"))
}

fn doc(text: &str) -> Result<Document, String> {
    match value(text)? {
        Bson::Document(d) => Ok(d),
        _ => Err(format!("se esperaba un objeto {{...}} y llegó «{}»", text.trim())),
    }
}

fn docs(text: &str) -> Result<Vec<Document>, String> {
    match value(text)? {
        Bson::Array(items) => items
            .into_iter()
            .map(|b| match b {
                Bson::Document(d) => Ok(d),
                _ => Err("el arreglo debe contener solo objetos {...}".to_string()),
            })
            .collect(),
        _ => Err(format!("se esperaba un arreglo [...] y llegó «{}»", text.trim())),
    }
}

fn string_arg(text: &str) -> Result<String, String> {
    match value(text)? {
        Bson::String(s) => Ok(s),
        _ => Err(format!("se esperaba un texto entre comillas y llegó «{}»", text.trim())),
    }
}

fn int_arg(text: &str) -> Result<i64, String> {
    match value(text)? {
        Bson::Int32(n) => Ok(i64::from(n)),
        Bson::Int64(n) => Ok(n),
        Bson::Double(f) if f.fract() == 0.0 => Ok(f as i64),
        _ => Err(format!("se esperaba un número entero y llegó «{}»", text.trim())),
    }
}

/// `.name(args)` calls after the collection, in order.
fn calls(mut rest: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return Ok(out);
        }
        let Some(after_dot) = rest.strip_prefix('.') else {
            return Err(format!(
                "no entiendo «{}»; se esperaba .metodo(...)",
                rest.chars().take(20).collect::<String>()
            ));
        };
        let name_len = after_dot
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(after_dot.len());
        let name = &after_dot[..name_len];
        let after_name = &after_dot[name_len..];
        if name.is_empty() || !after_name.starts_with('(') {
            return Err(format!("falta «(» después de .{name}"));
        }
        let close = matching(after_name, 0).ok_or_else(|| format!("falta cerrar el paréntesis de .{name}("))?;
        out.push((name.to_string(), after_name[1..close].to_string()));
        rest = &after_name[close + 1..];
    }
}

pub fn parse(input: &str) -> Result<Op, String> {
    let s = input.trim().trim_end_matches(';').trim();
    if s.is_empty() {
        return Err("Escribe una consulta.".to_string());
    }
    if s.starts_with('{') {
        return Ok(Op::Command(doc(s)?));
    }
    let Some(rest) = s.strip_prefix("db.") else {
        return Err(format!(
            "Escribe db.coleccion.find({{...}}), db.coleccion.aggregate([...]) o un comando {{...}}. Métodos: {SUPPORTED}."
        ));
    };

    let (coll, rest) = if let Some(r) = rest.strip_prefix("getCollection") {
        let close = matching(r, 0).ok_or("falta cerrar getCollection(")?;
        (string_arg(&r[1..close])?, &r[close + 1..])
    } else {
        let end = rest.find(['.', '(']).unwrap_or(rest.len());
        if rest[end..].starts_with('(') {
            let call = &rest[..end];
            return if call == "runCommand" {
                let close = matching(&rest[end..], 0).ok_or("falta cerrar runCommand(")?;
                Ok(Op::Command(doc(&rest[end + 1..end + close])?))
            } else {
                Err(format!(
                    "db.{call}() no está soportado; usa db.coleccion.metodo(...) o un comando {{...}}."
                ))
            };
        }
        (rest[..end].to_string(), &rest[end..])
    };
    if coll.is_empty() {
        return Err("Falta el nombre de la colección: db.coleccion.find(...)".to_string());
    }

    let mut calls = calls(rest)?.into_iter();
    let (method, args) = calls.next().ok_or("Falta el método: db.coleccion.find(...)")?;
    let args = split_args(&args);
    let arg = |i: usize| args.get(i).copied();
    let need = |i: usize, what: &str| arg(i).ok_or(format!("{method} necesita {what}"));

    let op = match method.as_str() {
        "find" | "findOne" => {
            let mut op = Op::Find {
                coll,
                filter: arg(0).map(doc).transpose()?.unwrap_or_default(),
                projection: arg(1).map(doc).transpose()?,
                sort: None,
                limit: (method == "findOne").then_some(1),
                skip: None,
            };
            for (name, a) in calls {
                let Op::Find { sort, limit, skip, .. } = &mut op else {
                    unreachable!()
                };
                match name.as_str() {
                    "sort" => *sort = Some(doc(&a)?),
                    "limit" => *limit = Some(int_arg(&a)?),
                    "skip" => *skip = Some(int_arg(&a)?.max(0) as u64),
                    "pretty" | "toArray" => {}
                    other => {
                        return Err(format!(
                            ".{other}() no está soportado después de find (usa sort, limit o skip)"
                        ))
                    }
                }
            }
            return Ok(op);
        }
        "aggregate" => Op::Aggregate {
            coll,
            pipeline: docs(need(0, "un arreglo con las etapas")?)?,
        },
        "countDocuments" => Op::Count {
            coll,
            filter: arg(0).map(doc).transpose()?,
        },
        "estimatedDocumentCount" => Op::Count { coll, filter: None },
        "distinct" => Op::Distinct {
            coll,
            field: string_arg(need(0, "el nombre del campo")?)?,
            filter: arg(1).map(doc).transpose()?,
        },
        "insertOne" => Op::InsertOne {
            coll,
            doc: doc(need(0, "un documento")?)?,
        },
        "insertMany" => Op::InsertMany {
            coll,
            docs: docs(need(0, "un arreglo de documentos")?)?,
        },
        "updateOne" | "updateMany" => Op::Update {
            coll,
            filter: doc(need(0, "un filtro")?)?,
            update: doc(need(1, "el documento de actualización")?)?,
            many: method == "updateMany",
        },
        "deleteOne" | "deleteMany" => Op::Delete {
            coll,
            filter: doc(need(0, "un filtro")?)?,
            many: method == "deleteMany",
        },
        other => return Err(format!("El método {other} no está soportado. Métodos: {SUPPORTED}.")),
    };
    if let Some((extra, _)) = calls.next() {
        return Err(format!(".{extra}() no se puede encadenar después de {method}"));
    }
    Ok(op)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    #[test]
    fn find_with_unquoted_keys_and_chain() {
        let op =
            parse("db.users.find({age: {$gt: 18}, name: 'Ana'}, {name: 1}).sort({age: -1}).limit(5).skip(2);").unwrap();
        assert_eq!(
            op,
            Op::Find {
                coll: "users".into(),
                filter: doc! { "age": { "$gt": 18 }, "name": "Ana" },
                projection: Some(doc! { "name": 1 }),
                sort: Some(doc! { "age": -1 }),
                limit: Some(5),
                skip: Some(2),
            }
        );
    }

    #[test]
    fn bare_find_and_find_one() {
        match parse("db.c.find()").unwrap() {
            Op::Find { filter, limit, .. } => assert!(filter.is_empty() && limit.is_none()),
            o => panic!("{o:?}"),
        }
        match parse("db.c.findOne({a: 1})").unwrap() {
            Op::Find { limit, .. } => assert_eq!(limit, Some(1)),
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn shell_helpers_become_extended_json() {
        let op = parse(r#"db.c.find({_id: ObjectId("650000000000000000000001"), n: NumberLong("7"), d: ISODate("2024-01-02T03:04:05Z")})"#).unwrap();
        let Op::Find { filter, .. } = op else { panic!() };
        assert!(matches!(filter.get("_id"), Some(Bson::ObjectId(_))));
        assert_eq!(filter.get("n"), Some(&Bson::Int32(7)).or(Some(&Bson::Int64(7))));
        assert!(matches!(filter.get("d"), Some(Bson::DateTime(_))));
    }

    #[test]
    fn helpers_inside_strings_are_left_alone() {
        let Op::Find { filter, .. } = parse(r#"db.c.find({t: "ObjectId(1)"})"#).unwrap() else {
            panic!()
        };
        assert_eq!(filter.get_str("t").unwrap(), "ObjectId(1)");
    }

    #[test]
    fn commas_and_parens_inside_strings_do_not_split_arguments() {
        let Op::Update {
            filter, update, many, ..
        } = parse(r#"db.c.updateMany({a: "x, (y)"}, {$set: {b: "), ("}})"#).unwrap()
        else {
            panic!()
        };
        assert!(many);
        assert_eq!(filter.get_str("a").unwrap(), "x, (y)");
        assert_eq!(update, doc! { "$set": { "b": "), (" } });
    }

    #[test]
    fn aggregate_count_distinct_and_writes() {
        assert!(matches!(
            parse("db.c.aggregate([{$match: {a: 1}}, {$group: {_id: '$b', n: {$sum: 1}}}])").unwrap(),
            Op::Aggregate { pipeline, .. } if pipeline.len() == 2
        ));
        assert!(matches!(
            parse("db.c.countDocuments({a: 1})").unwrap(),
            Op::Count { filter: Some(_), .. }
        ));
        assert!(matches!(parse("db.c.distinct('a')").unwrap(), Op::Distinct { field, .. } if field == "a"));
        assert!(parse("db.c.insertOne({a: 1})").unwrap().is_write());
        assert!(parse("db.c.deleteMany({})").unwrap().is_write());
        assert!(!parse("db.c.find()").unwrap().is_write());
        assert!(
            matches!(parse("db.c.insertMany([{a: 1}, {a: 2}])").unwrap(), Op::InsertMany { docs, .. } if docs.len() == 2)
        );
    }

    #[test]
    fn destructive_operations_are_flagged() {
        let d = |t: &str| destructive(&parse(t).unwrap());
        assert!(!d("db.c.deleteMany({})").is_empty());
        assert!(!d("db.c.deleteOne({})").is_empty());
        assert!(!d("db.c.deleteMany({a: 1})").is_empty());
        assert!(d("db.c.deleteOne({a: 1})").is_empty());
        assert!(!d("db.c.updateMany({}, {$set: {a: 1}})").is_empty());
        assert!(d("db.c.updateMany({a: 1}, {$set: {a: 2}})").is_empty());
        assert!(d("db.c.updateOne({}, {$set: {a: 2}})").is_empty());
        assert!(!d("{dropDatabase: 1}").is_empty());
        assert!(!d("{drop: 'c'}").is_empty());
        assert!(d("{ping: 1}").is_empty());
        assert!(d("db.c.find()").is_empty());
        assert!(d("db.c.insertOne({a: 1})").is_empty());
    }

    #[test]
    fn get_collection_and_commands() {
        assert!(
            matches!(parse(r#"db.getCollection("mi-col").find()"#).unwrap(), Op::Find { coll, .. } if coll == "mi-col")
        );
        assert_eq!(parse("{ping: 1}").unwrap(), Op::Command(doc! { "ping": 1 }));
        assert_eq!(
            parse("db.runCommand({ping: 1})").unwrap(),
            Op::Command(doc! { "ping": 1 })
        );
    }

    #[test]
    fn errors_are_explained_not_panicked() {
        for bad in [
            "",
            "   ",
            "select 1",
            "db.",
            "db.c",
            "db.c.find(",
            "db.c.find({a: })",
            "db.c.find().foo()",
            "db.c.nope()",
            "db.c.updateOne({a: 1})",
            "db.c.find([1])",
            "db.c.aggregate({})",
            "db.c.find({a:1}) extra",
            "db.getCollectionNames()",
            "db.c.find({'a: 1})",
        ] {
            let e = parse(bad).unwrap_err();
            assert!(!e.is_empty(), "{bad}");
        }
    }
}
