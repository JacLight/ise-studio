//! How a material looks (colour, metalness, roughness), for files that carry
//! colour. The table is the one the view renders with (apps/web/src/model.ts),
//! read from its source so an export looks like the screen and the two never
//! drift apart.

use super::Look;
use std::sync::OnceLock;

const MODEL_TS: &str = include_str!("../../../../../apps/web/src/model.ts");

struct Table {
    /// Material families in the view's order (exact names first, then families).
    looks: Vec<(String, Look)>,
    /// Colour words a material name may carry ("PLA (Bambu, black)"), longest first.
    words: Vec<(String, [f64; 3])>,
}

pub fn hex(s: &str) -> Option<[f64; 3]> {
    let h = s.trim().strip_prefix('#')?;
    let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok().map(|b| b as f64 / 255.0);
    if h.len() == 6 { Some([byte(0)?, byte(2)?, byte(4)?]) } else { None }
}

/// The body of `const NAME … = { … };` in the view's source.
fn block<'a>(src: &'a str, decl: &str) -> &'a str {
    let Some(i) = src.find(decl) else { return "" };
    let rest = &src[i..];
    let open = rest.find('{').map(|k| k + 1).unwrap_or(0);
    let close = rest.find("\n};").unwrap_or(rest.len());
    &rest[open.min(close)..close]
}

/// A key at the start of a line: `"fr4 green mask":` or `pla:`.
fn key(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if let Some(r) = line.strip_prefix('"') {
        let end = r.find('"')?;
        Some((&r[..end], r[end + 1..].trim_start().strip_prefix(':')?))
    } else {
        let (k, rest) = line.split_once(':')?;
        k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_').then_some((k, rest))
    }
}

fn number(fields: &str, name: &str) -> Option<f64> {
    let i = fields.find(&format!("{name}:"))?;
    let v = fields[i + name.len() + 1..].trim_start();
    let end = v.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).unwrap_or(v.len());
    v[..end].parse().ok()
}

fn table() -> &'static Table {
    static T: OnceLock<Table> = OnceLock::new();
    T.get_or_init(|| {
        let mut looks = Vec::new();
        for line in block(MODEL_TS, "const LOOKS").lines() {
            let Some((k, rest)) = key(line) else { continue };
            let Some(c) = rest.find("color:").and_then(|i| rest[i + 6..].split('"').nth(1)).and_then(hex) else { continue };
            looks.push((
                k.to_string(),
                Look { color: c, metalness: number(rest, "metalness").unwrap_or(0.1), roughness: number(rest, "roughness").unwrap_or(0.6), opacity: number(rest, "opacity").unwrap_or(1.0) },
            ));
        }
        let mut words: Vec<(String, [f64; 3])> =
            block(MODEL_TS, "const COLOUR_WORDS").lines().filter_map(|l| key(l).and_then(|(k, r)| r.split('"').nth(1).and_then(hex).map(|c| (k.to_string(), c)))).collect();
        words.sort_by_key(|(w, _)| std::cmp::Reverse(w.len()));
        Table { looks, words }
    })
}

/// Whether `word` appears in `text` as a whole word.
fn has_word(text: &str, word: &str) -> bool {
    let wordy = |c: char| c.is_alphanumeric() || c == '_';
    text.match_indices(word).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + word.len()..].chars().next();
        !before.is_some_and(wordy) && !after.is_some_and(wordy)
    })
}

/// The look of a material by name, as the view renders it: an exact name,
/// else the family it names ("6061 aluminium"), with a colour in the name
/// ("PLA (Bambu, black)", "#c8372f") winning over the family's own.
pub fn look(name: &str) -> Look {
    let t = table();
    let key = name.to_lowercase();
    let base = t.looks.iter().find(|(k, _)| *k == key).or_else(|| t.looks.iter().find(|(k, _)| key.contains(k.as_str()))).map(|(_, l)| l.clone());
    let mut look = base.unwrap_or(Look { color: [0xc4 as f64 / 255.0, 0xc9 as f64 / 255.0, 0xd4 as f64 / 255.0], metalness: 0.2, roughness: 0.5, opacity: 1.0 });
    let from_hex = key.match_indices('#').find_map(|(i, _)| key.get(i..i + 7).and_then(hex).filter(|_| !key[i + 7..].starts_with(|c: char| c.is_ascii_hexdigit())));
    if let Some(c) = from_hex.or_else(|| t.words.iter().find(|(w, _)| has_word(&key, w)).map(|(_, c)| *c)) {
        look.color = c;
    }
    look
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_match_the_view() {
        let t = table();
        assert!(t.looks.len() > 40 && t.words.len() > 20, "{} looks, {} words read from model.ts", t.looks.len(), t.words.len());
        let close = |a: [f64; 3], h: &str| a.iter().zip(hex(h).unwrap()).all(|(x, y)| (x - y).abs() < 1e-9);
        assert!(close(look("PLA").color, "#4f7dd9"));
        assert!(close(look("6061 Aluminium").color, "#c8d2da") && look("6061 aluminium").metalness > 0.5);
        assert!(close(look("PLA (Bambu, black)").color, "#232427"), "a colour word wins");
        assert!(close(look("PETG dark grey").color, "#4a4d52"), "“dark grey” before “grey”");
        assert!(close(look("Imported #c8372f").color, "#c8372f"));
        assert!(close(look("Stainless steel").color, "#d0d5d9"), "the longer family listed first wins");
    }
}
