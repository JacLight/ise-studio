//! A small XML reader (elements, attributes, text; no DTD processing), enough
//! for the EDA formats that use XML: Eagle boards and schematics, IPC-2581.
//! Entities: the five predefined ones and numeric character references.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Element>,
    pub text: String,
}

impl Element {
    pub fn attr(&self, k: &str) -> Option<&str> {
        self.attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
    }
    pub fn num(&self, k: &str) -> Option<f64> {
        self.attr(k).and_then(|v| v.trim().parse().ok())
    }
    pub fn numor(&self, k: &str, d: f64) -> f64 {
        self.num(k).unwrap_or(d)
    }
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }
    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
    /// The first element at a path of child names ("drawing/board/plain").
    pub fn path(&self, p: &str) -> Option<&Element> {
        let mut e = self;
        for part in p.split('/') {
            e = e.child(part)?;
        }
        Some(e)
    }
    /// Every descendant with this name, depth first.
    pub fn descendants<'a>(&'a self, name: &str, out: &mut Vec<&'a Element>) {
        for c in &self.children {
            if c.name == name {
                out.push(c);
            }
            c.descendants(name, out);
        }
    }
}

pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(j) = rest.find(';') else { break };
        let ent = &rest[1..j];
        let ch = match ent {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[j + 1..];
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

pub fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '&' => o.push_str("&amp;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' => {}
            c => o.push(c),
        }
    }
    o
}

/// Parses a document into its root element.
pub fn parse(text: &str) -> Result<Element, String> {
    let b = text.as_bytes();
    let mut i = 0;
    let mut stack: Vec<Element> = vec![Element { name: "#document".into(), ..Default::default() }];
    while i < b.len() {
        if b[i] == b'<' {
            if text[i..].starts_with("<!--") {
                i = text[i..].find("-->").map(|j| i + j + 3).ok_or("unclosed comment")?;
            } else if text[i..].starts_with("<![CDATA[") {
                let j = text[i..].find("]]>").ok_or("unclosed CDATA")?;
                stack.last_mut().unwrap().text.push_str(&text[i + 9..i + j]);
                i += j + 3;
            } else if text[i..].starts_with("<?") {
                i = text[i..].find("?>").map(|j| i + j + 2).ok_or("unclosed declaration")?;
            } else if text[i..].starts_with("<!") {
                // DOCTYPE, possibly with an internal subset in brackets.
                let mut depth = 0;
                let mut j = i;
                while j < b.len() {
                    match b[j] {
                        b'[' => depth += 1,
                        b']' => depth -= 1,
                        b'>' if depth <= 0 => break,
                        _ => {}
                    }
                    j += 1;
                }
                i = j + 1;
            } else if text[i..].starts_with("</") {
                let j = text[i..].find('>').ok_or("unclosed end tag")? + i;
                let name = text[i + 2..j].trim();
                let e = stack.pop().ok_or("unbalanced end tag")?;
                if e.name != name {
                    return Err(format!("</{name}> closes <{}>", e.name));
                }
                stack.last_mut().ok_or("unbalanced end tag")?.children.push(e);
                i = j + 1;
            } else {
                // Start tag: name, attributes, maybe self-closing.
                let mut j = i + 1;
                while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' && b[j] != b'/' {
                    j += 1;
                }
                let mut e = Element { name: text[i + 1..j].to_string(), ..Default::default() };
                loop {
                    while j < b.len() && b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j >= b.len() {
                        return Err("the file ends inside a tag".into());
                    }
                    if b[j] == b'>' {
                        stack.push(e);
                        i = j + 1;
                        break;
                    }
                    if b[j] == b'/' {
                        let close = text[j..].find('>').ok_or("unclosed tag")? + j;
                        stack.last_mut().unwrap().children.push(e);
                        i = close + 1;
                        break;
                    }
                    let k0 = j;
                    while j < b.len() && b[j] != b'=' && !b[j].is_ascii_whitespace() && b[j] != b'>' {
                        j += 1;
                    }
                    let key = text[k0..j].to_string();
                    while j < b.len() && b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j < b.len() && b[j] == b'=' {
                        j += 1;
                        while j < b.len() && b[j].is_ascii_whitespace() {
                            j += 1;
                        }
                        let q = b[j];
                        if q == b'"' || q == b'\'' {
                            let end = text[j + 1..].find(q as char).ok_or("unclosed attribute")? + j + 1;
                            e.attrs.push((key, unescape(&text[j + 1..end])));
                            j = end + 1;
                        } else {
                            let v0 = j;
                            while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' {
                                j += 1;
                            }
                            e.attrs.push((key, unescape(&text[v0..j])));
                        }
                    } else {
                        e.attrs.push((key, String::new()));
                    }
                }
            }
        } else {
            let j = text[i..].find('<').map(|j| i + j).unwrap_or(b.len());
            let t = &text[i..j];
            if !t.trim().is_empty() {
                stack.last_mut().unwrap().text.push_str(&unescape(t));
            }
            i = j;
        }
    }
    if stack.len() != 1 {
        return Err(format!("<{}> is never closed", stack.last().map(|e| e.name.as_str()).unwrap_or("?")));
    }
    let doc = stack.pop().unwrap();
    doc.children.into_iter().next().ok_or_else(|| "no root element".into())
}

/// Builds XML text with indentation.
pub struct Writer {
    pub out: String,
    stack: Vec<String>,
}

impl Default for Writer {
    fn default() -> Self {
        Self::new()
    }
}

impl Writer {
    pub fn new() -> Writer {
        Writer { out: "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n".into(), stack: vec![] }
    }
    fn indent(&mut self) {
        for _ in 0..self.stack.len() {
            self.out.push_str("  ");
        }
    }
    fn tag(&mut self, name: &str, attrs: &[(&str, String)]) {
        self.indent();
        self.out.push('<');
        self.out.push_str(name);
        for (k, v) in attrs {
            self.out.push_str(&format!(" {k}=\"{}\"", escape(v)));
        }
    }
    pub fn open(&mut self, name: &str, attrs: &[(&str, String)]) {
        self.tag(name, attrs);
        self.out.push_str(">\n");
        self.stack.push(name.to_string());
    }
    pub fn empty(&mut self, name: &str, attrs: &[(&str, String)]) {
        self.tag(name, attrs);
        self.out.push_str("/>\n");
    }
    pub fn text(&mut self, name: &str, attrs: &[(&str, String)], text: &str) {
        self.tag(name, attrs);
        self.out.push_str(&format!(">{}</{name}>\n", escape(text)));
    }
    pub fn close(&mut self) {
        let n = self.stack.pop().unwrap_or_default();
        self.indent();
        self.out.push_str(&format!("</{n}>\n"));
    }
    pub fn finish(mut self) -> String {
        while !self.stack.is_empty() {
            self.close();
        }
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_elements_attributes_and_entities() {
        let x = parse("<?xml version=\"1.0\"?>\n<!DOCTYPE eagle SYSTEM \"eagle.dtd\">\n<a k=\"1 &amp; 2\"><!-- c --><b x='3'/><c>t &lt;x&gt;</c></a>").unwrap();
        assert_eq!(x.name, "a");
        assert_eq!(x.attr("k"), Some("1 & 2"));
        assert_eq!(x.child("b").unwrap().num("x"), Some(3.0));
        assert_eq!(x.child("c").unwrap().text, "t <x>");
        assert!(parse("<a><b></a>").is_err());
        let mut w = Writer::new();
        w.open("r", &[("v", "a\"b".into())]);
        w.text("t", &[], "<1>");
        let s = w.finish();
        let back = parse(&s).unwrap();
        assert_eq!(back.attr("v"), Some("a\"b"));
        assert_eq!(back.child("t").unwrap().text, "<1>");
    }
}
