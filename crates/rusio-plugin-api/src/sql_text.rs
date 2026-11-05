//! SQL text helpers for the console: syntax highlighting and formatting.
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Keyword,
    Function,
    String,
    Number,
    Comment,
    Quoted,
    Plain,
}

const KEYWORDS: &[&str] = &[
    "ADD",
    "ALL",
    "ALTER",
    "AND",
    "ANY",
    "AS",
    "ASC",
    "BEGIN",
    "BETWEEN",
    "BY",
    "CASE",
    "CAST",
    "CHECK",
    "COLUMN",
    "COMMIT",
    "CONSTRAINT",
    "CREATE",
    "CROSS",
    "CURRENT_DATE",
    "CURRENT_TIMESTAMP",
    "DATABASE",
    "DEFAULT",
    "DELETE",
    "DESC",
    "DISTINCT",
    "DROP",
    "ELSE",
    "END",
    "EXCEPT",
    "EXISTS",
    "EXPLAIN",
    "FALSE",
    "FETCH",
    "FILTER",
    "FOR",
    "FOREIGN",
    "FROM",
    "FULL",
    "GRANT",
    "GROUP",
    "HAVING",
    "ILIKE",
    "IN",
    "INDEX",
    "INNER",
    "INSERT",
    "INTERSECT",
    "INTO",
    "IS",
    "JOIN",
    "KEY",
    "LATERAL",
    "LEFT",
    "LIKE",
    "LIMIT",
    "NATURAL",
    "NOT",
    "NULL",
    "OFFSET",
    "ON",
    "OR",
    "ORDER",
    "OUTER",
    "OVER",
    "PARTITION",
    "PRIMARY",
    "REFERENCES",
    "REPLACE",
    "RETURNING",
    "REVOKE",
    "RIGHT",
    "ROLLBACK",
    "ROW",
    "ROWS",
    "SCHEMA",
    "SELECT",
    "SET",
    "TABLE",
    "THEN",
    "TO",
    "TRIGGER",
    "TRUE",
    "TRUNCATE",
    "UNION",
    "UNIQUE",
    "UPDATE",
    "USING",
    "VALUES",
    "VIEW",
    "WHEN",
    "WHERE",
    "WINDOW",
    "WITH",
];

fn is_keyword(word: &str) -> bool {
    let up = word.to_ascii_uppercase();
    KEYWORDS.binary_search(&up.as_str()).is_ok()
}

/// Splits `sql` into `(kind, text)` pieces covering the whole input, in order.
pub fn tokenize(sql: &str) -> Vec<(Kind, &str)> {
    let b = sql.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut plain_start = 0;

    macro_rules! flush_plain {
        ($end:expr) => {
            if plain_start < $end {
                out.push((Kind::Plain, &sql[plain_start..$end]));
            }
        };
    }

    while i < b.len() {
        let c = b[i];
        let start = i;
        let kind = if c == b'-' && b.get(i + 1) == Some(&b'-') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            Some(Kind::Comment)
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                i += 1;
            }
            i = (i + 2).min(b.len());
            Some(Kind::Comment)
        } else if c == b'\'' || c == b'"' || c == b'`' {
            i += 1;
            while i < b.len() {
                if b[i] == c {
                    // A doubled quote is an escaped quote.
                    if b.get(i + 1) == Some(&c) {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            Some(if c == b'\'' { Kind::String } else { Kind::Quoted })
        } else if c.is_ascii_digit() {
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            Some(Kind::Number)
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'$') {
                i += 1;
            }
            let word = &sql[start..i];
            // `name(` is a function call unless it is a keyword such as IN (
            let rest = sql[i..].trim_start_matches([' ', '\t']);
            if is_keyword(word) {
                Some(Kind::Keyword)
            } else if rest.starts_with('(') {
                Some(Kind::Function)
            } else {
                None
            }
        } else {
            i += 1;
            None
        };

        match kind {
            Some(k) => {
                flush_plain!(start);
                out.push((k, &sql[start..i]));
                plain_start = i;
            }
            None => {
                // Stay inside plain text; make sure `i` sits on a char boundary.
                while i < b.len() && !sql.is_char_boundary(i) {
                    i += 1;
                }
            }
        }
    }
    flush_plain!(b.len());
    out
}

fn color(kind: Kind) -> Color32 {
    match kind {
        Kind::Keyword => Color32::from_rgb(197, 134, 255),
        Kind::Function => Color32::from_rgb(110, 190, 255),
        Kind::String => Color32::from_rgb(152, 211, 121),
        Kind::Number => Color32::from_rgb(244, 164, 96),
        Kind::Comment => Color32::from_rgb(107, 114, 128),
        Kind::Quoted => Color32::from_rgb(120, 220, 210),
        Kind::Plain => Color32::from_rgb(229, 231, 235),
    }
}

/// `sql = false` keeps strings, numbers and comments but not SQL keywords (other query languages).
pub fn highlight(sql: &str, font: FontId, is_sql: bool) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (kind, text) in tokenize(sql) {
        let kind = match kind {
            Kind::Keyword | Kind::Function if !is_sql => Kind::Plain,
            k => k,
        };
        job.append(
            text,
            0.0,
            TextFormat {
                font_id: font.clone(),
                color: color(kind),
                italics: kind == Kind::Comment,
                ..Default::default()
            },
        );
    }
    job
}

/// Why running `sql` could destroy data (empty = nothing risky found). Looks at each statement:
/// any `DROP`, `TRUNCATE`, and `DELETE`/`UPDATE` without a `WHERE`. Text in strings, quoted
/// identifiers and comments is ignored.
pub fn destructive_sql(sql: &str) -> Vec<String> {
    let mut reasons: Vec<String> = Vec::new();
    let mut add = |r: &str| {
        if !reasons.iter().any(|x| x == r) {
            reasons.push(r.to_string());
        }
    };
    let mut words: Vec<String> = Vec::new();

    let finish = |words: &mut Vec<String>, add: &mut dyn FnMut(&str)| {
        let has = |w: &str| words.iter().any(|x| x == w);
        match words.first().map(String::as_str) {
            Some("TRUNCATE") => add("TRUNCATE"),
            Some("DELETE") if !has("WHERE") => add("DELETE sin WHERE"),
            Some("UPDATE") if !has("WHERE") => add("UPDATE sin WHERE"),
            _ => {}
        }
        if has("DROP") {
            add("DROP");
        }
        words.clear();
    };

    for (kind, text) in tokenize(sql) {
        if matches!(kind, Kind::String | Kind::Quoted | Kind::Comment) {
            continue;
        }
        let mut cur = String::new();
        for ch in text.chars().chain(std::iter::once(' ')) {
            if ch.is_alphanumeric() || ch == '_' {
                cur.push(ch.to_ascii_uppercase());
                continue;
            }
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
            if ch == ';' {
                finish(&mut words, &mut add);
            }
        }
    }
    finish(&mut words, &mut add);
    reasons
}

/// Re-indents the statements and upper-cases keywords.
pub fn format_sql(sql: &str) -> String {
    let opts = sqlformat::FormatOptions {
        indent: sqlformat::Indent::Spaces(2),
        uppercase: Some(true),
        lines_between_queries: 1,
        ..Default::default()
    };
    sqlformat::format(sql, &sqlformat::QueryParams::None, &opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(sql: &str) -> Vec<(Kind, String)> {
        tokenize(sql).into_iter().map(|(k, t)| (k, t.to_string())).collect()
    }

    #[test]
    fn keyword_list_is_sorted_for_binary_search() {
        let mut sorted = KEYWORDS.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, KEYWORDS);
    }

    #[test]
    fn tokens_cover_the_input_exactly() {
        let sql = "SELECT a, 'x''y' /* c */ FROM t -- fin\nWHERE n >= 10.5 AND \"Col\" = ñandú";
        let joined: String = tokenize(sql).iter().map(|(_, t)| *t).collect();
        assert_eq!(joined, sql);
    }

    #[test]
    fn classifies_the_basic_kinds() {
        let k = kinds("select count(*) from t where a = 'o''k' and b > 3 -- x");
        let find = |text: &str| k.iter().find(|(_, t)| t == text).map(|(kind, _)| *kind);
        assert_eq!(find("select"), Some(Kind::Keyword));
        assert_eq!(find("count"), Some(Kind::Function));
        assert_eq!(find("'o''k'"), Some(Kind::String));
        assert_eq!(find("3"), Some(Kind::Number));
        assert_eq!(find("-- x"), Some(Kind::Comment));
        assert_eq!(find("t"), None); // part of a plain run
    }

    #[test]
    fn keywords_followed_by_parens_stay_keywords_and_unterminated_input_is_safe() {
        assert_eq!(kinds("IN (1)")[0].0, Kind::Keyword);
        // Unterminated string / comment must not panic or lose text.
        for sql in ["'abc", "/* abc", "\"abc", "-- abc", "é'ñ"] {
            let joined: String = tokenize(sql).iter().map(|(_, t)| *t).collect();
            assert_eq!(joined, sql);
        }
    }

    #[test]
    fn destructive_statements_are_detected() {
        assert_eq!(destructive_sql("DROP TABLE t"), ["DROP"]);
        assert_eq!(destructive_sql("drop database x;"), ["DROP"]);
        assert_eq!(destructive_sql("ALTER TABLE t DROP COLUMN c"), ["DROP"]);
        assert_eq!(destructive_sql("TRUNCATE t"), ["TRUNCATE"]);
        assert_eq!(destructive_sql("delete from t"), ["DELETE sin WHERE"]);
        assert_eq!(destructive_sql("UPDATE t SET a = 1"), ["UPDATE sin WHERE"]);
        // Only the risky statement of a batch counts, and each reason is listed once.
        assert_eq!(
            destructive_sql("SELECT 1; DELETE FROM a; DELETE FROM b; UPDATE t SET x=1 WHERE id=2"),
            ["DELETE sin WHERE"]
        );
    }

    #[test]
    fn safe_statements_are_not_flagged() {
        for sql in [
            "SELECT * FROM t",
            "DELETE FROM t WHERE id = 1",
            "UPDATE t SET a = 1 WHERE b = 2",
            "INSERT INTO t VALUES (1)",
            "CREATE TABLE t (a int REFERENCES p ON DELETE CASCADE)",
            "INSERT INTO t VALUES (1) ON CONFLICT (a) DO UPDATE SET b = 2",
            "SELECT 'DROP TABLE t', \"drop\" FROM x -- DELETE FROM y",
            "/* DROP TABLE t */ SELECT 1",
            "",
        ] {
            assert!(destructive_sql(sql).is_empty(), "{sql}");
        }
    }

    #[test]
    fn formatting_indents_and_uppercases() {
        let f = format_sql("select a,b from t where a=1 and b=2");
        assert!(f.contains("SELECT"));
        assert!(f.contains('\n'));
        assert!(f.contains("FROM"));
    }
}
