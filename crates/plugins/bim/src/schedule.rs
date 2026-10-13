//! Schedules and quantity take-off: every element, space and zone as a row
//! with its class, level, type, material, base quantities (declared by the
//! program or read from IFC) and quantities measured from its geometry
//! (volume; openings in a host; a space's area, perimeter and volume),
//! filtered by class and level and totalled per level, class, type or material.

use crate::{ifc_write::quantity_kind, num_text, Model, PValue, Psets};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Write as _;

#[derive(Clone, Debug, Serialize)]
pub struct Row {
    pub name: String,
    pub class: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predefined: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    pub quantities: BTreeMap<String, f64>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub psets: Psets,
}

#[derive(Clone, Debug, Serialize)]
pub struct Total {
    /// The grouping's values, in the order asked ("Ground floor", "IfcWall").
    pub key: Vec<String>,
    pub count: usize,
    /// Sums of every additive quantity (lengths, areas, volumes, weights, counts).
    pub sums: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Schedule {
    pub rows: Vec<Row>,
    /// Quantity names in column order.
    pub columns: Vec<String>,
    /// Units of each column ("mm", "m²", "m³", "kg", "").
    pub units: BTreeMap<String, String>,
    pub group_by: Vec<String>,
    pub groups: Vec<Total>,
    pub total: Total,
}

#[derive(Clone, Debug, Default)]
pub struct Query {
    /// Classes to include ("IfcWall", or "wall"); empty: all.
    pub classes: Vec<String>,
    /// Levels to include; empty: all.
    pub levels: Vec<String>,
    /// Any of "level", "class", "type", "material", "predefined".
    pub group_by: Vec<String>,
}

const ORDER: &[&str] = &[
    "Length", "Width", "Height", "Depth", "Thickness", "Perimeter", "GrossPerimeter", "OverallWidth", "OverallHeight", "NominalDiameter", "BarLength", "NumberOfRisers", "RiserHeight", "TreadLength",
    "GrossSideArea", "NetSideArea", "GrossArea", "NetArea", "GrossFloorArea", "NetFloorArea", "Area", "GrossVolume", "NetVolume", "Volume", "Count", "TotalLength", "Weight", "Openings", "Spaces",
];

/// Not summed: sizes of one thing.
fn additive(name: &str) -> bool {
    !matches!(name, "Width" | "Height" | "Depth" | "Thickness" | "OverallWidth" | "OverallHeight" | "NominalDiameter" | "RiserHeight" | "TreadLength" | "Slope" | "Pitch" | "BarLength" | "CrossSectionArea")
}

pub fn unit(name: &str) -> &'static str {
    match quantity_kind(name) {
        "area" => "m²",
        "volume" => "m³",
        "weight" => "kg",
        "count" => "",
        _ if name == "Openings" || name == "Spaces" || name == "NumberOfRisers" => "",
        _ => "mm",
    }
}

fn matches_class(class: &str, wanted: &[String]) -> bool {
    wanted.is_empty() || wanted.iter().any(|w| crate::class_name(w).eq_ignore_ascii_case(class))
}

/// Every element, space and zone of the model as rows.
pub fn rows(model: &Model) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut openings: BTreeMap<&str, usize> = BTreeMap::new();
    for o in &model.openings {
        *openings.entry(o.host.as_str()).or_default() += 1;
    }
    for e in &model.elements {
        let i = &e.info;
        let mut q = i.quantities.clone();
        q.insert("Volume".into(), e.volume / 1e9);
        for k in ["OverallWidth", "OverallHeight", "NominalDiameter", "BarLength", "NumberOfRisers", "RiserHeight", "TreadLength"] {
            if let Some(PValue::Num(n)) = i.attributes.get(k) {
                q.entry(k.into()).or_insert(*n);
            }
        }
        if let Some(n) = openings.get(e.name.as_str()) {
            q.insert("Openings".into(), *n as f64);
        }
        rows.push(Row {
            name: e.name.clone(),
            class: if i.class.is_empty() { "IfcBuildingElementProxy".into() } else { i.class.clone() },
            predefined: i.predefined.clone(),
            type_name: i.type_name.clone(),
            level: i.level.clone(),
            tag: i.tag.clone(),
            material: e.material.clone(),
            host: i.host.clone(),
            quantities: q,
            psets: i.psets.clone(),
        });
    }
    for s in &model.spaces {
        let mut q = BTreeMap::new();
        q.insert("NetFloorArea".into(), s.area());
        q.insert("Perimeter".into(), s.perimeter());
        q.insert("Height".into(), s.height);
        q.insert("NetVolume".into(), s.area() * s.height / 1000.0);
        rows.push(Row {
            name: s.name.clone(),
            class: "IfcSpace".into(),
            predefined: s.predefined.clone(),
            type_name: s.long_name.clone(),
            level: s.level.clone(),
            tag: None,
            material: None,
            host: None,
            quantities: q,
            psets: s.psets.clone(),
        });
    }
    for z in &model.zones {
        let mut q = BTreeMap::new();
        let members: Vec<&crate::Space> = model.spaces.iter().filter(|s| z.spaces.contains(&s.name)).collect();
        q.insert("NetFloorArea".into(), members.iter().map(|s| s.area()).sum());
        q.insert("NetVolume".into(), members.iter().map(|s| s.area() * s.height / 1000.0).sum());
        q.insert("Spaces".into(), members.len() as f64);
        rows.push(Row { name: z.name.clone(), class: "IfcZone".into(), predefined: None, type_name: None, level: None, tag: None, material: None, host: None, quantities: q, psets: z.psets.clone() });
    }
    rows
}

fn key_of(r: &Row, by: &str) -> String {
    match by {
        "level" => r.level.clone().unwrap_or_else(|| "—".into()),
        "class" => r.class.clone(),
        "type" => r.type_name.clone().unwrap_or_else(|| "—".into()),
        "material" => r.material.clone().unwrap_or_else(|| "—".into()),
        "predefined" => r.predefined.clone().unwrap_or_else(|| "—".into()),
        _ => "—".into(),
    }
}

fn sum(rows: &[&Row], key: Vec<String>) -> Total {
    let mut sums: BTreeMap<String, f64> = BTreeMap::new();
    for r in rows {
        for (k, v) in &r.quantities {
            if additive(k) {
                *sums.entry(k.clone()).or_default() += v;
            }
        }
    }
    Total { key, count: rows.len(), sums }
}

/// The schedule for a query: rows in level order, then class, then name.
pub fn schedule(model: &Model, q: &Query) -> Schedule {
    let level_index = |l: &Option<String>| l.as_ref().and_then(|l| model.levels.iter().position(|x| &x.name == l)).unwrap_or(usize::MAX);
    let mut rows: Vec<Row> = rows(model).into_iter().filter(|r| matches_class(&r.class, &q.classes) && (q.levels.is_empty() || r.level.as_ref().is_some_and(|l| q.levels.iter().any(|w| w.eq_ignore_ascii_case(l))))).collect();
    rows.sort_by(|a, b| level_index(&a.level).cmp(&level_index(&b.level)).then(a.class.cmp(&b.class)).then_with(|| natural(&a.name, &b.name)));
    let mut names: Vec<String> = rows.iter().flat_map(|r| r.quantities.keys().cloned()).collect();
    names.sort();
    names.dedup();
    names.sort_by_key(|n| (ORDER.iter().position(|o| o == n).unwrap_or(ORDER.len()), n.clone()));
    let units = names.iter().map(|n| (n.clone(), unit(n).to_string())).collect();
    let mut groups: Vec<Total> = Vec::new();
    if !q.group_by.is_empty() {
        let mut keys: Vec<Vec<String>> = Vec::new();
        for r in &rows {
            let k: Vec<String> = q.group_by.iter().map(|b| key_of(r, b)).collect();
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
        for k in keys {
            let members: Vec<&Row> = rows.iter().filter(|r| q.group_by.iter().map(|b| key_of(r, b)).collect::<Vec<_>>() == k).collect();
            groups.push(sum(&members, k));
        }
    }
    let total = sum(&rows.iter().collect::<Vec<_>>(), vec!["Total".into()]);
    Schedule { rows, columns: names, units, group_by: q.group_by.clone(), groups, total }
}

fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    // Numbers inside names compare by value: "Wall 2" before "Wall 10".
    let split = |s: &str| -> Vec<(String, u64)> {
        let mut out = Vec::new();
        let mut text = String::new();
        let mut num = String::new();
        for c in s.chars() {
            if c.is_ascii_digit() {
                num.push(c);
            } else {
                if !num.is_empty() {
                    out.push((std::mem::take(&mut text), num.parse().unwrap_or(0)));
                    num.clear();
                }
                text.push(c);
            }
        }
        out.push((text, num.parse().unwrap_or(0)));
        out
    };
    split(a).cmp(&split(b))
}

pub fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

fn q_text(v: f64, unit: &str) -> String {
    match unit {
        "m²" | "m³" => format!("{v:.3}"),
        "kg" => format!("{v:.2}"),
        "mm" => format!("{v:.0}"),
        _ => num_text(v),
    }
}

/// The schedule as CSV: one line per element (or per group when grouped),
/// then a total line. `psets` adds a column per property ("Pset.Name").
pub fn csv(s: &Schedule, psets: bool) -> String {
    let mut out = String::new();
    let head = |n: &String| {
        let u = &s.units[n];
        if u.is_empty() { n.clone() } else { format!("{n} ({u})") }
    };
    let qcols: Vec<String> = s.columns.iter().map(head).collect();
    if !s.group_by.is_empty() {
        let mut h: Vec<String> = s.group_by.iter().map(|g| capital(g)).collect();
        h.push("Count".into());
        h.extend(s.columns.iter().filter(|c| additive(c)).map(head));
        let _ = writeln!(out, "{}", h.iter().map(|x| csv_field(x)).collect::<Vec<_>>().join(","));
        for g in s.groups.iter().chain(std::iter::once(&s.total)) {
            let mut line: Vec<String> = g.key.iter().map(|k| csv_field(k)).collect();
            while line.len() < s.group_by.len() {
                line.push(String::new());
            }
            line.push(g.count.to_string());
            line.extend(s.columns.iter().filter(|c| additive(c)).map(|c| g.sums.get(c).map(|v| q_text(*v, &s.units[c])).unwrap_or_default()));
            let _ = writeln!(out, "{}", line.join(","));
        }
        return out;
    }
    let mut props: Vec<String> = vec![];
    if psets {
        for r in &s.rows {
            for (set, p) in &r.psets {
                for k in p.keys() {
                    let c = format!("{set}.{k}");
                    if !props.contains(&c) {
                        props.push(c);
                    }
                }
            }
        }
    }
    let mut h = vec!["Level".to_string(), "Class".into(), "Type".into(), "Name".into(), "Tag".into(), "Material".into(), "Host".into()];
    h.extend(qcols);
    h.extend(props.iter().cloned());
    let _ = writeln!(out, "{}", h.iter().map(|x| csv_field(x)).collect::<Vec<_>>().join(","));
    for r in &s.rows {
        let mut line = vec![
            csv_field(r.level.as_deref().unwrap_or("")),
            csv_field(&r.class),
            csv_field(r.type_name.as_deref().unwrap_or("")),
            csv_field(&r.name),
            csv_field(r.tag.as_deref().unwrap_or("")),
            csv_field(r.material.as_deref().unwrap_or("")),
            csv_field(r.host.as_deref().unwrap_or("")),
        ];
        line.extend(s.columns.iter().map(|c| r.quantities.get(c).map(|v| q_text(*v, &s.units[c])).unwrap_or_default()));
        for p in &props {
            let (set, k) = p.split_once('.').unwrap();
            line.push(csv_field(&r.psets.get(set).and_then(|m| m.get(k)).map(PValue::text).unwrap_or_default()));
        }
        let _ = writeln!(out, "{}", line.join(","));
    }
    let mut line = vec!["Total".to_string(), String::new(), String::new(), format!("{} items", s.total.count), String::new(), String::new(), String::new()];
    line.extend(s.columns.iter().map(|c| if additive(c) { s.total.sums.get(c).map(|v| q_text(*v, &s.units[c])).unwrap_or_default() } else { String::new() }));
    let _ = writeln!(out, "{}", line.join(","));
    out
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}
