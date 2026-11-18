//! Printing results. JSON is the default because agents and scripts read it; `--output table`
//! is for people. Errors always go to stderr as one JSON object.

use clap::ValueEnum;
use serde_json::Value;

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum, Default)]
pub enum Format {
    #[default]
    Json,
    Table,
}

/// Columns for table output: a header and a dotted path into each row (`price.monthlyUsd`).
pub type Columns = &'static [(&'static str, &'static str)];

pub fn print(v: &Value, format: Format, cols: Columns) {
    match format {
        Format::Json => println!("{}", serde_json::to_string_pretty(v).unwrap_or_default()),
        Format::Table => print!("{}", table(v, cols)),
    }
}

fn lookup<'a>(v: &'a Value, path: &str) -> &'a Value {
    path.split('.')
        .fold(v, |v, k| v.get(k).unwrap_or(&Value::Null))
}

fn cell(v: &Value) -> String {
    match v {
        Value::Null => "-".into(),
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(cell).collect::<Vec<_>>().join(", "),
        Value::Object(_) => v.to_string(),
        other => other.to_string(),
    }
}

/// Renders rows as aligned columns, or an object as `key  value` lines.
pub fn table(v: &Value, cols: Columns) -> String {
    let rows: Vec<&Value> = match v {
        Value::Array(a) => a.iter().collect(),
        Value::Object(_) if !cols.is_empty() => vec![v],
        Value::Object(o) => {
            let w = o.keys().map(|k| k.len()).max().unwrap_or(0);
            return o
                .iter()
                .map(|(k, v)| format!("{k:<w$}  {}\n", cell(v)))
                .collect();
        }
        other => return format!("{}\n", cell(other)),
    };
    if rows.is_empty() {
        return "no results\n".into();
    }
    let cols: Vec<(&str, &str)> = if cols.is_empty() {
        match rows[0] {
            Value::Object(o) => o.keys().map(|k| (k.as_str(), k.as_str())).collect(),
            _ => vec![("value", "")],
        }
    } else {
        cols.to_vec()
    };
    let grid: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            cols.iter()
                .map(|(_, p)| {
                    if p.is_empty() {
                        cell(r)
                    } else {
                        cell(lookup(r, p))
                    }
                })
                .collect()
        })
        .collect();
    let widths: Vec<usize> = cols
        .iter()
        .enumerate()
        .map(|(i, (h, _))| {
            grid.iter()
                .map(|r| r[i].chars().count())
                .max()
                .unwrap_or(0)
                .max(h.len())
        })
        .collect();
    let line = |cells: Vec<String>| -> String {
        let mut s = cells
            .iter()
            .zip(&widths)
            .map(|(c, w)| format!("{c}{}", " ".repeat(w.saturating_sub(c.chars().count()))))
            .collect::<Vec<_>>()
            .join("  ");
        s.truncate(s.trim_end().len());
        s + "\n"
    };
    let mut out = line(cols.iter().map(|(h, _)| h.to_string()).collect());
    for r in grid {
        out += &line(r);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn table_follows_dotted_paths_and_aligns() {
        let v = json!([{ "id": "pod_web", "price": { "monthlyUsd": 8.76 } }, { "id": "p2", "price": null }]);
        let t = table(&v, &[("id", "id"), ("usd/mo", "price.monthlyUsd")]);
        assert_eq!(t, "id       usd/mo\npod_web  8.76\np2       -\n");
    }

    #[test]
    fn empty_list_says_so() {
        assert_eq!(table(&json!([]), &[("id", "id")]), "no results\n");
    }
}
