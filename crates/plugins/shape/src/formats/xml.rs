//! Just enough XML for 3MF models and SVG drawings: elements and their
//! attributes as a stream of start and end tags (text, comments, processing
//! instructions and doctypes are skipped).

pub enum Event<'a> {
    /// A start tag: its name (namespace prefix dropped) and attributes. An
    /// empty element (`<a/>`) is followed by its own `End`.
    Start { name: &'a str, attrs: Vec<(&'a str, String)> },
    End { name: &'a str },
}

/// The tags of a document, in order.
pub fn events(text: &str) -> Result<Vec<Event<'_>>, String> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut i = 0;
    while let Some(off) = text[i..].find('<') {
        i += off;
        let rest = &text[i..];
        if rest.starts_with("<!--") {
            i += rest.find("-->").ok_or("an XML comment is not closed")? + 3;
        } else if rest.starts_with("<![CDATA[") {
            i += rest.find("]]>").ok_or("an XML CDATA section is not closed")? + 3;
        } else if rest.starts_with("<?") {
            i += rest.find("?>").ok_or("an XML declaration is not closed")? + 2;
        } else if rest.starts_with("<!") {
            // A doctype, possibly with an internal subset in brackets.
            let mut depth = 0;
            let mut j = 0;
            for (k, c) in rest.char_indices() {
                match c {
                    '[' => depth += 1,
                    ']' => depth -= 1,
                    '>' if depth == 0 => {
                        j = k;
                        break;
                    }
                    _ => {}
                }
            }
            i += j + 1;
        } else if let Some(close) = rest.strip_prefix("</") {
            let end = close.find('>').ok_or("an XML end tag is not closed")?;
            out.push(Event::End { name: local(close[..end].trim()) });
            i += end + 3;
        } else {
            // A start tag: name, then attributes until `>` or `/>` (quotes may hold either).
            let mut j = i + 1;
            while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' && b[j] != b'/' {
                j += 1;
            }
            let name = local(&text[i + 1..j]);
            let mut attrs = Vec::new();
            let empty;
            loop {
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                match b.get(j) {
                    None => return Err(format!("the XML tag <{name}> is not closed")),
                    Some(b'>') => {
                        empty = false;
                        j += 1;
                        break;
                    }
                    Some(b'/') => {
                        empty = true;
                        j = j + text[j..].find('>').ok_or("an XML tag is not closed")? + 1;
                        break;
                    }
                    _ => {}
                }
                let k = j;
                while j < b.len() && b[j] != b'=' && !b[j].is_ascii_whitespace() && b[j] != b'>' && b[j] != b'/' {
                    j += 1;
                }
                let key = &text[k..j];
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if b.get(j) != Some(&b'=') {
                    // An attribute with no value (HTML style): kept as empty.
                    attrs.push((key, String::new()));
                    continue;
                }
                j += 1;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                let quote = *b.get(j).ok_or("an XML attribute is cut short")?;
                if quote != b'"' && quote != b'\'' {
                    return Err(format!("the attribute {key} of <{name}> is not quoted"));
                }
                let v = text[j + 1..].find(quote as char).ok_or("an XML attribute is not closed")?;
                attrs.push((key, unescape(&text[j + 1..j + 1 + v])));
                j += v + 2;
            }
            out.push(Event::Start { name, attrs });
            if empty {
                out.push(Event::End { name });
            }
            i = j;
        }
    }
    Ok(out)
}

/// A name without its namespace prefix (`p:path` → `path`, `svg:rect` → `rect`).
pub fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// An attribute by name (prefix ignored).
pub fn attr<'a>(attrs: &'a [(&str, String)], key: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| *k == key || local(k) == key).map(|(_, v)| v.as_str())
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';') else { break };
        let entity = &rest[1..end];
        let c = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_attributes_and_entities() {
        let doc = r#"<?xml version="1.0"?><!-- c --><!DOCTYPE svg [<!ENTITY x "y">]><svg:svg a="1 &amp; 2" b='x>y'><g/><path d="M0 0"></path></svg:svg>"#;
        let ev = events(doc).unwrap();
        let names: Vec<String> = ev
            .iter()
            .map(|e| match e {
                Event::Start { name, .. } => format!("+{name}"),
                Event::End { name } => format!("-{name}"),
            })
            .collect();
        assert_eq!(names, ["+svg", "+g", "-g", "+path", "-path", "-svg"]);
        let Event::Start { attrs, .. } = &ev[0] else { panic!() };
        assert_eq!(attr(attrs, "a"), Some("1 & 2"));
        assert_eq!(attr(attrs, "b"), Some("x>y"));
    }
}
