//! The official KiCad libraries, on demand: footprints
//! (gitlab.com/kicad/libraries/kicad-footprints) and symbols
//! (gitlab.com/kicad/libraries/kicad-symbols), downloaded once into a cache
//! folder on this computer, never into a project or the repository.
//!
//! They are CC-BY-SA 4.0 with KiCad's design exception: a board made with
//! them is the designer's own and carries no licence; the libraries
//! themselves, redistributed, keep theirs. So Forma reads them where they
//! were downloaded and copies only what a board uses into that board (as
//! `pcb.footprint.define` and a symbol instance's `definition` do with any
//! part), each copy naming the library it came from.
//!
//! - [`Library::search`] finds parts by name, description and keywords in an
//!   index built from the files once (kept next to them).
//! - [`Library::footprint`] reads a `.kicad_mod` into a Forma footprint
//!   (through the board importer's reader: pads exactly, y up).
//! - [`Library::symbol`] reads a symbol into a [`SymbolDef`]: pins with their
//!   numbers (which are the footprint's pad numbers in KiCad's libraries),
//!   electrical types and sides, its drawing per unit, default footprint,
//!   and the parent's body for a derived (`extends`) symbol.
//!
//! The download is an archive of each repository (about 12 MB each, 180 and
//! 270 MB unpacked), fetched with `curl` and unpacked with `tar`, both part of
//! macOS, Windows 10+ and Linux; `FORMA_KICAD_LIBRARY` puts the cache
//! elsewhere (or points at a KiCad library checkout already on disk).

use crate::footprints::Footprint;
use crate::sexpr::{self, Sexp};
use crate::symbols::{Dir, Fill, Gfx, PinDef, PinType, SymbolDef};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

pub const FOOTPRINTS_URL: &str = "https://gitlab.com/kicad/libraries/kicad-footprints/-/archive/master/kicad-footprints-master.tar.gz";
pub const SYMBOLS_URL: &str = "https://gitlab.com/kicad/libraries/kicad-symbols/-/archive/master/kicad-symbols-master.tar.gz";
pub const LICENCE: &str = "KiCad libraries by the KiCad library contributors, CC-BY-SA 4.0 with the KiCad libraries design exception (designs made with them carry no licence obligation)";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Repo {
    Footprints,
    Symbols,
}

impl Repo {
    pub fn folder(self) -> &'static str {
        match self {
            Repo::Footprints => "kicad-footprints",
            Repo::Symbols => "kicad-symbols",
        }
    }
    pub fn url(self) -> &'static str {
        match self {
            Repo::Footprints => FOOTPRINTS_URL,
            Repo::Symbols => SYMBOLS_URL,
        }
    }
}

/// Where the libraries are cached: `FORMA_KICAD_LIBRARY`, else the user's cache folder.
pub fn cache_root() -> PathBuf {
    if let Ok(p) = std::env::var("FORMA_KICAD_LIBRARY") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| ".".into());
    if cfg!(target_os = "macos") {
        return Path::new(&home).join("Library/Caches/Forma/kicad");
    }
    if let Ok(x) = std::env::var("XDG_CACHE_HOME") {
        if !x.is_empty() {
            return Path::new(&x).join("forma/kicad");
        }
    }
    if cfg!(windows) {
        if let Ok(x) = std::env::var("LOCALAPPDATA") {
            return Path::new(&x).join("Forma/kicad");
        }
    }
    Path::new(&home).join(".cache/forma/kicad")
}

/// One library item in the index.
#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    /// `Library:Name`, as KiCad names it.
    pub id: String,
    pub library: String,
    pub name: String,
    pub description: String,
    pub keywords: String,
    /// Symbols: their default footprint (`Package_TO_SOT_SMD:SOT-23-5`), if any.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub footprint: String,
    /// Symbols: the footprint name patterns that fit (`SOT?23?5*`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub filters: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub datasheet: String,
    /// Pads (footprints) or pins (symbols).
    pub count: usize,
    pub repo: Repo,
}

/// What is downloaded.
#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub root: String,
    pub footprints: Option<String>,
    pub symbols: Option<String>,
    pub licence: &'static str,
}

/// The KiCad libraries in one folder (`<root>/kicad-footprints`, `<root>/kicad-symbols`).
#[derive(Clone, Debug)]
pub struct Library {
    pub root: PathBuf,
}

static INDEX: OnceLock<Mutex<HashMap<PathBuf, Arc<Vec<Entry>>>>> = OnceLock::new();
static FETCH: Mutex<()> = Mutex::new(());

impl Default for Library {
    fn default() -> Self {
        Library { root: cache_root() }
    }
}

impl Library {
    pub fn at(root: impl Into<PathBuf>) -> Library {
        Library { root: root.into() }
    }

    pub fn dir(&self, repo: Repo) -> PathBuf {
        self.root.join(repo.folder())
    }

    pub fn present(&self, repo: Repo) -> bool {
        self.dir(repo).is_dir()
    }

    pub fn status(&self) -> Status {
        let info = |r: Repo| -> Option<String> {
            if !self.present(r) {
                return None;
            }
            Some(std::fs::read_to_string(self.dir(r).join(".forma-source")).map(|s| s.trim().to_string()).unwrap_or_else(|_| "on disk".into()))
        };
        Status { root: self.root.display().to_string(), footprints: info(Repo::Footprints), symbols: info(Repo::Symbols), licence: LICENCE }
    }

    /// Downloads (again) and unpacks one repository; the old copy stays until the new one is in place.
    pub fn fetch(&self, repo: Repo) -> Result<PathBuf, String> {
        let _guard = FETCH.lock().unwrap_or_else(|e| e.into_inner());
        std::fs::create_dir_all(&self.root).map_err(|e| format!("cannot make {}: {e}", self.root.display()))?;
        let stage = self.root.join(format!(".download-{}-{}", repo.folder(), std::process::id()));
        let _ = std::fs::remove_dir_all(&stage);
        std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
        let archive = stage.join("archive.tar.gz");
        let out = std::process::Command::new("curl")
            .args(["-fsSL", "--retry", "2", "--connect-timeout", "20", "-o"])
            .arg(&archive)
            .arg(repo.url())
            .output()
            .map_err(|e| format!("cannot run curl to download the KiCad {}: {e}", repo.folder()))?;
        if !out.status.success() {
            let _ = std::fs::remove_dir_all(&stage);
            return Err(format!("downloading {} failed: {}", repo.url(), String::from_utf8_lossy(&out.stderr).trim()));
        }
        let out = std::process::Command::new("tar").arg("-xzf").arg(&archive).arg("-C").arg(&stage).output().map_err(|e| format!("cannot run tar: {e}"))?;
        if !out.status.success() {
            let _ = std::fs::remove_dir_all(&stage);
            return Err(format!("unpacking the KiCad {} failed: {}", repo.folder(), String::from_utf8_lossy(&out.stderr).trim()));
        }
        let _ = std::fs::remove_file(&archive);
        let unpacked = std::fs::read_dir(&stage).map_err(|e| e.to_string())?.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.is_dir()).ok_or("the archive held no folder")?;
        let when = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let _ = std::fs::write(unpacked.join(".forma-source"), format!("{} (downloaded {})\n{LICENCE}\n", repo.url(), iso_date(when)));
        let dest = self.dir(repo);
        let old = self.root.join(format!(".old-{}-{}", repo.folder(), std::process::id()));
        if dest.exists() {
            std::fs::rename(&dest, &old).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&unpacked, &dest).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir_all(&old);
        let _ = std::fs::remove_dir_all(&stage);
        let _ = std::fs::remove_file(self.root.join(index_file(repo)));
        if let Some(m) = INDEX.get() {
            m.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.root.join(index_file(repo)));
        }
        Ok(dest)
    }

    /// The repository's folder, downloading it the first time.
    pub fn ensure(&self, repo: Repo) -> Result<PathBuf, String> {
        if self.present(repo) { Ok(self.dir(repo)) } else { self.fetch(repo) }
    }

    /// A footprint file's text by `Library:Name`.
    pub fn footprint_text(&self, id: &str) -> Result<String, String> {
        let (lib, name) = split_id(id)?;
        let dir = self.ensure(Repo::Footprints)?;
        let path = dir.join(format!("{lib}.pretty")).join(format!("{name}.kicad_mod"));
        std::fs::read_to_string(&path).map_err(|_| format!("no footprint {id} in the KiCad library (search with `./forma pcb parts <words>`)"))
    }

    /// A footprint in Forma's terms, named `Library:Name`, with notes on anything approximated.
    pub fn footprint(&self, id: &str) -> Result<(Footprint, Vec<String>), String> {
        let text = self.footprint_text(id)?;
        let (lib, _) = split_id(id)?;
        let (mut fp, notes) = crate::kicad::read_footprint_file(&text, Some(lib))?;
        fp.name = id.to_string();
        fp.source = format!("KiCad library {id} ({LICENCE}); body height estimated");
        Ok((fp, notes))
    }

    /// A symbol by `Library:Name`, its parent's drawing and pins filled in when it extends one.
    pub fn symbol(&self, id: &str) -> Result<SymbolDef, String> {
        let (lib, name) = split_id(id)?;
        let dir = self.ensure(Repo::Symbols)?;
        let (sym, parent) = load_symbol(&dir, lib, name)?;
        Ok(symbol_def(lib, &sym, parent.as_ref()))
    }

    /// Every footprint or symbol, indexed once (the index is kept beside the files).
    pub fn index(&self, repo: Repo) -> Result<Arc<Vec<Entry>>, String> {
        let file = self.root.join(index_file(repo));
        let memo = INDEX.get_or_init(|| Mutex::new(HashMap::new()));
        if let Some(v) = memo.lock().unwrap_or_else(|e| e.into_inner()).get(&file) {
            return Ok(v.clone());
        }
        let dir = self.ensure(repo)?;
        let entries = match std::fs::read_to_string(&file).ok().and_then(|t| read_index(&t, repo)) {
            Some(v) => v,
            None => {
                let v = match repo {
                    Repo::Footprints => scan_footprints(&dir),
                    Repo::Symbols => scan_symbols(&dir),
                };
                let _ = std::fs::write(&file, write_index(&v));
                v
            }
        };
        let arc = Arc::new(entries);
        memo.lock().unwrap_or_else(|e| e.into_inner()).insert(file, arc.clone());
        Ok(arc)
    }

    /// Parts matching every word of `query` (name, library, description, keywords), best first.
    pub fn search(&self, query: &str, repo: Repo, limit: usize) -> Result<Vec<Entry>, String> {
        let index = self.index(repo)?;
        Ok(search(&index, query, limit))
    }
}

fn index_file(repo: Repo) -> String {
    format!("index-{}.tsv", repo.folder())
}

pub fn split_id(id: &str) -> Result<(&str, &str), String> {
    let (lib, name) = id.split_once(':').ok_or_else(|| format!("“{id}” is not a KiCad library id (Library:Name, e.g. Package_SO:SOIC-8_3.9x4.9mm_P1.27mm)"))?;
    let ok = |s: &str| !s.is_empty() && !s.contains(['/', '\\']) && s != "." && s != "..";
    if !ok(lib) || !ok(name) {
        return Err(format!("“{id}” is not a KiCad library id"));
    }
    Ok((lib, name))
}

/// Days since 1970 → YYYY-MM-DD (civil calendar).
fn iso_date(secs: u64) -> String {
    let z = (secs / 86400) as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

// ---- Index --------------------------------------------------------------------------

fn clean_field(s: &str) -> String {
    s.chars().map(|c| if c == '\t' || c == '\n' || c == '\r' { ' ' } else { c }).collect()
}

fn write_index(v: &[Entry]) -> String {
    let mut s = String::from("forma-kicad-index 1\n");
    for e in v {
        s.push_str(&[e.library.as_str(), &e.name, &e.description, &e.keywords, &e.footprint, &e.filters, &e.datasheet, &e.count.to_string()].map(clean_field).join("\t"));
        s.push('\n');
    }
    s
}

fn read_index(text: &str, repo: Repo) -> Option<Vec<Entry>> {
    let mut lines = text.lines();
    if lines.next()? != "forma-kicad-index 1" {
        return None;
    }
    let mut out = Vec::new();
    for l in lines {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() < 8 {
            return None;
        }
        out.push(Entry { id: format!("{}:{}", f[0], f[1]), library: f[0].into(), name: f[1].into(), description: f[2].into(), keywords: f[3].into(), footprint: f[4].into(), filters: f[5].into(), datasheet: f[6].into(), count: f[7].parse().unwrap_or(0), repo });
    }
    (!out.is_empty()).then_some(out)
}

/// The quoted string after `(key ` in raw S-expression text (no full parse: indexing reads 20 000 files).
fn quoted_after(text: &str, key: &str) -> Option<String> {
    let at = text.find(key)? + key.len();
    let rest = text[at..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n') => out.push(' '),
                Some(o) => out.push(o),
                None => break,
            },
            '"' => return Some(out),
            c => out.push(c),
        }
    }
    None
}

fn sorted_dirs(dir: &Path, suffix: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).map(|r| r.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().and_then(|x| x.to_str()) == Some(suffix)).collect()).unwrap_or_default();
    v.sort();
    v
}

fn scan_footprints(dir: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    for lib_dir in sorted_dirs(dir, "pretty") {
        let lib = lib_dir.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
        for f in sorted_dirs(&lib_dir, "kicad_mod") {
            let Ok(text) = std::fs::read_to_string(&f) else { continue };
            let name = f.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            out.push(Entry {
                id: format!("{lib}:{name}"),
                library: lib.clone(),
                name,
                description: quoted_after(&text, "(descr ").unwrap_or_default(),
                keywords: quoted_after(&text, "(tags ").unwrap_or_default(),
                footprint: String::new(),
                filters: String::new(),
                datasheet: String::new(),
                count: text.matches("(pad ").count(),
                repo: Repo::Footprints,
            });
        }
    }
    out
}

fn scan_symbols(dir: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut parents: Vec<(usize, String)> = Vec::new();
    let prop = |text: &str, k: &str| quoted_after(text, &format!("(property \"{k}\" ")).unwrap_or_default();
    for lib_dir in sorted_dirs(dir, "kicad_symdir") {
        let lib = lib_dir.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
        for f in sorted_dirs(&lib_dir, "kicad_sym") {
            let Ok(text) = std::fs::read_to_string(&f) else { continue };
            let name = f.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            if let Some(p) = quoted_after(&text, "(extends ") {
                parents.push((out.len(), format!("{lib}:{p}")));
            }
            out.push(Entry {
                id: format!("{lib}:{name}"),
                library: lib.clone(),
                name,
                description: prop(&text, "Description"),
                keywords: prop(&text, "ki_keywords"),
                footprint: prop(&text, "Footprint"),
                filters: prop(&text, "ki_fp_filters"),
                datasheet: prop(&text, "Datasheet").trim_start_matches('~').to_string(),
                count: text.matches("(pin ").count(),
                repo: Repo::Symbols,
            });
        }
    }
    // A derived symbol has its parent's pins.
    let counts: HashMap<String, usize> = out.iter().map(|e| (e.id.clone(), e.count)).collect();
    for (i, p) in parents {
        if out[i].count == 0 {
            out[i].count = counts.get(&p).copied().unwrap_or(0);
        }
    }
    out
}

/// Ranks entries for a query: every word must appear somewhere; name hits count most.
pub fn search(index: &[Entry], query: &str, limit: usize) -> Vec<Entry> {
    let words: Vec<String> = query.split(|c: char| c.is_whitespace() || c == ',').filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect();
    if words.is_empty() {
        return vec![];
    }
    let mut scored: Vec<(i64, &Entry)> = Vec::new();
    for e in index {
        let name = e.name.to_lowercase();
        let lib = e.library.to_lowercase();
        let rest = format!("{} {}", e.description, e.keywords).to_lowercase();
        let mut score = 0i64;
        let mut all = true;
        for w in &words {
            if name == *w {
                score += 100;
            } else if name.starts_with(w.as_str()) {
                score += 40;
            } else if name.contains(w.as_str()) {
                score += 25;
            } else if lib.contains(w.as_str()) {
                score += 10;
            } else if rest.split(|c: char| !c.is_alphanumeric() && c != '-' && c != '.').any(|t| t == w) {
                score += 8;
            } else if rest.contains(w.as_str()) {
                score += 3;
            } else {
                all = false;
                break;
            }
        }
        if all {
            // Shorter names are the plainer variants (SOT-23 before SOT-23_Handsoldering).
            score -= (name.len() as i64) / 8;
            scored.push((score, e));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
    scored.into_iter().take(limit).map(|(_, e)| e.clone()).collect()
}

// ---- Symbols --------------------------------------------------------------------------

/// The symbol and, for a derived one, its parent (same library), from the per-symbol
/// files of the current libraries or a KiCad 6–8 single-file library.
fn load_symbol(dir: &Path, lib: &str, name: &str) -> Result<(Sexp, Option<Sexp>), String> {
    let read = |n: &str| -> Result<Sexp, String> {
        let per_symbol = dir.join(format!("{lib}.kicad_symdir")).join(format!("{n}.kicad_sym"));
        let single = dir.join(format!("{lib}.kicad_sym"));
        let text = std::fs::read_to_string(&per_symbol).or_else(|_| std::fs::read_to_string(&single)).map_err(|_| format!("no symbol {lib}:{n} in the KiCad library (search with `./forma pcb parts <words>`)"))?;
        let top = sexpr::parse(&text)?;
        let root = top.iter().find(|e| e.is("kicad_symbol_lib")).ok_or("not a KiCad symbol library")?;
        root.find_all("symbol").find(|s| s.arg_text(0) == Some(n)).cloned().ok_or_else(|| format!("no symbol {lib}:{n} in the KiCad library"))
    };
    let sym = read(name)?;
    let parent = match sym.find("extends").and_then(|e| e.arg_text(0)) {
        Some(p) => Some(read(p)?),
        None => None,
    };
    Ok((sym, parent))
}

/// KiCad text markup to plain text: `V_{IN}` → `VIN`, `~{RESET}` → `/RESET`.
pub fn plain(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if (c == '_' || c == '^' || c == '~') && chars.peek() == Some(&'{') {
            chars.next();
            if c == '~' {
                out.push('/');
            }
            continue;
        }
        if c == '}' {
            continue;
        }
        out.push(c);
    }
    out
}

fn category_for(lib: &str) -> &'static str {
    let l = lib.to_ascii_lowercase();
    if l == "device" || l.starts_with("resistor") || l.starts_with("capacitor") || l.starts_with("inductor") {
        "Passive"
    } else if l.starts_with("diode") || l.starts_with("led") {
        "Diode"
    } else if l.starts_with("transistor") {
        "Transistor"
    } else if l.starts_with("connector") {
        "Connector"
    } else if l.starts_with("regulator") || l.starts_with("power_") || l.starts_with("battery") || l.starts_with("converter") {
        "Power IC"
    } else if l.starts_with("rf_module") || l.starts_with("module") {
        "Module"
    } else if l == "power" {
        "Power"
    } else if l.starts_with("switch") || l.starts_with("mechanical") || l.starts_with("jumper") {
        "Misc"
    } else {
        "IC"
    }
}

fn etype(s: &str) -> PinType {
    match s {
        "input" => PinType::Input,
        "output" => PinType::Output,
        "bidirectional" => PinType::Bidirectional,
        "tri_state" => PinType::TriState,
        "power_in" => PinType::PowerIn,
        "power_out" => PinType::PowerOut,
        "open_collector" => PinType::OpenCollector,
        "open_emitter" => PinType::OpenEmitter,
        "no_connect" => PinType::NoConnect,
        "unspecified" | "free" => PinType::Unspecified,
        _ => PinType::Passive,
    }
}

fn flip(x: f64, y: f64) -> [f64; 2] {
    [x, if y == 0.0 { 0.0 } else { -y }]
}

fn hidden(e: &Sexp) -> bool {
    e.has_atom("hide") || e.find("hide").is_some_and(|h| h.arg_text(0) != Some("no"))
}

fn fill_of(e: &Sexp) -> Fill {
    match e.find("fill").and_then(|f| f.value("type")) {
        Some("background") | Some("color") => Fill::Body,
        Some("outline") => Fill::Outline,
        _ => Fill::None,
    }
}

fn xy_list(e: &Sexp) -> Vec<[f64; 2]> {
    e.find("pts").map(|p| p.find_all("xy").map(|q| flip(q.arg_num(0).unwrap_or(0.0), q.arg_num(1).unwrap_or(0.0))).collect()).unwrap_or_default()
}

fn point(e: &Sexp, key: &str) -> Option<[f64; 2]> {
    let v = e.nums(key);
    (v.len() >= 2).then(|| flip(v[0], v[1]))
}

/// A three-point arc as a polyline (sheet coordinates).
fn arc_points(a: [f64; 2], m: [f64; 2], b: [f64; 2]) -> Vec<[f64; 2]> {
    let d = 2.0 * (a[0] * (m[1] - b[1]) + m[0] * (b[1] - a[1]) + b[0] * (a[1] - m[1]));
    if d.abs() < 1e-12 {
        return vec![a, b];
    }
    let sq = |p: [f64; 2]| p[0] * p[0] + p[1] * p[1];
    let c = [(sq(a) * (m[1] - b[1]) + sq(m) * (b[1] - a[1]) + sq(b) * (a[1] - m[1])) / d, (sq(a) * (b[0] - m[0]) + sq(m) * (a[0] - b[0]) + sq(b) * (m[0] - a[0])) / d];
    let r = ((a[0] - c[0]).powi(2) + (a[1] - c[1]).powi(2)).sqrt();
    let ang = |p: [f64; 2]| (p[1] - c[1]).atan2(p[0] - c[0]);
    let (a0, am, a1) = (ang(a), ang(m), ang(b));
    let tau = std::f64::consts::TAU;
    let ccw = |from: f64, to: f64| (to - from).rem_euclid(tau);
    // Sweep from a to b the way that passes through m.
    let span = if ccw(a0, am) <= ccw(a0, a1) { ccw(a0, a1) } else { ccw(a0, a1) - tau };
    let n = ((span.abs() / (std::f64::consts::PI / 16.0)).ceil() as usize).max(2);
    (0..=n).map(|i| {
        let t = a0 + span * i as f64 / n as f64;
        [c[0] + r * t.cos(), c[1] + r * t.sin()]
    }).collect()
}

fn graphic(g: &Sexp) -> Option<Gfx> {
    match g.head()? {
        "rectangle" => Some(Gfx::Rect { a: point(g, "start")?, b: point(g, "end")?, fill: fill_of(g) }),
        "polyline" => {
            let pts = xy_list(g);
            if pts.len() < 2 {
                return None;
            }
            let fill = fill_of(g);
            Some(if fill != Fill::None && pts.len() >= 3 { Gfx::Poly { pts, fill } } else { Gfx::Line { pts, width: 0.0 } })
        }
        "bezier" => {
            let pts = xy_list(g);
            (pts.len() >= 2).then(|| Gfx::Line { pts, width: 0.0 })
        }
        "circle" => Some(Gfx::Circle { c: point(g, "center")?, r: g.value_num("radius")?, fill: fill_of(g) }),
        "arc" => {
            let pts = arc_points(point(g, "start")?, point(g, "mid")?, point(g, "end")?);
            Some(Gfx::Line { pts, width: 0.0 })
        }
        "text" => {
            let at = point(g, "at")?;
            let size = g.find("effects").and_then(|e| e.find("font")).map(|f| f.nums("size")).and_then(|v| v.first().copied()).unwrap_or(1.27);
            Some(Gfx::Text { at, text: plain(g.arg_text(0)?), size })
        }
        _ => None,
    }
}

fn property<'a>(s: &'a Sexp, key: &str) -> Option<&'a str> {
    s.find_all("property").find(|p| p.arg_text(0) == Some(key)).and_then(|p| p.arg_text(1)).filter(|v| !v.is_empty() && *v != "~")
}

/// Libraries of generic parts, whose value is not a part number.
const GENERIC: [&str; 8] = ["Device", "power", "Connector", "Connector_Generic", "Connector_Generic_MountingPin", "Switch", "Mechanical", "Jumper"];

/// A KiCad symbol as a Forma library symbol named `Library:Name`. A derived symbol takes
/// its parent's pins and drawing and its own fields.
pub fn symbol_def(lib: &str, sym: &Sexp, parent: Option<&Sexp>) -> SymbolDef {
    let name = sym.arg_text(0).unwrap_or("").to_string();
    let body = parent.unwrap_or(sym);
    let body_name = body.arg_text(0).unwrap_or("").to_string();
    let field = |k: &str| property(sym, k).or_else(|| parent.and_then(|p| property(p, k))).map(str::to_string);
    let reference = field("Reference").unwrap_or_else(|| "U".into());
    let value = field("Value").unwrap_or_else(|| name.clone());
    let power = body.find("power").is_some() || reference.starts_with('#');
    let mut units: BTreeMap<u32, Vec<Gfx>> = BTreeMap::new();
    let mut common: Vec<Gfx> = Vec::new();
    let mut pins: Vec<PinDef> = Vec::new();
    for unit in body.find_all("symbol") {
        // "<name>_<unit>_<style>": style 2 is the De Morgan alternate, not drawn.
        let id = unit.arg_text(0).unwrap_or("");
        let tail = id.strip_prefix(&body_name).unwrap_or(id).trim_start_matches('_');
        let mut it = tail.split('_');
        let u: u32 = it.next().and_then(|x| x.parse().ok()).unwrap_or(1);
        let style: u32 = it.next().and_then(|x| x.parse().ok()).unwrap_or(1);
        if style > 1 {
            continue;
        }
        for g in unit.args() {
            if g.is("pin") {
                let at = g.nums("at");
                let (x, y, angle) = (*at.first().unwrap_or(&0.0), *at.get(1).unwrap_or(&0.0), *at.get(2).unwrap_or(&0.0));
                // KiCad's angle points from the connection point into the body.
                let side = match ((angle.rem_euclid(360.0) / 90.0).round() as i64) % 4 {
                    0 => Dir::Left,
                    1 => Dir::Down,
                    2 => Dir::Right,
                    _ => Dir::Up,
                };
                let number = g.find("number").and_then(|n| n.arg_text(0)).unwrap_or("").to_string();
                let pname = g.find("name").and_then(|n| n.arg_text(0)).map(plain).unwrap_or_default();
                pins.push(PinDef {
                    number,
                    name: if pname.is_empty() { "~".into() } else { pname },
                    at: flip(x, y),
                    side,
                    length: g.value_num("length").unwrap_or(2.54),
                    etype: etype(g.arg_text(0).unwrap_or("passive")),
                    hidden: hidden(g),
                    unit: u,
                    pads: vec![],
                });
            } else if let Some(gfx) = graphic(g) {
                if u == 0 { common.push(gfx) } else { units.entry(u).or_default().push(gfx) }
            }
        }
    }
    let n_units = units.keys().copied().max().unwrap_or(1).max(pins.iter().map(|p| p.unit).max().unwrap_or(1)).max(1);
    let drawn: Vec<Vec<Gfx>> = (1..=n_units).map(|u| common.iter().cloned().chain(units.get(&u).cloned().unwrap_or_default()).collect()).collect();
    if n_units == 1 {
        for p in &mut pins {
            p.unit = 0;
        }
    }
    let pin_numbers_hidden = body.find("pin_numbers").is_some_and(hidden);
    let pin_names_hidden = body.find("pin_names").is_some_and(hidden);
    let mpn = if GENERIC.contains(&lib) || power { None } else { Some(value.clone()) };
    let known = mpn.as_deref().and_then(|m| crate::sourcing::KNOWN.iter().find(|k| k.mpn.eq_ignore_ascii_case(m) || k.mpn.to_ascii_uppercase().starts_with(&m.to_ascii_uppercase())));
    let description = field("Description").or_else(|| field("ki_description")).unwrap_or_default();
    let datasheet = field("Datasheet").filter(|d| d.starts_with("http"));
    SymbolDef {
        name: format!("{lib}:{name}"),
        description: match datasheet {
            Some(d) => format!("{description} (KiCad {lib}:{name}; datasheet {d})"),
            None => format!("{description} (KiCad {lib}:{name})"),
        },
        prefix: reference.trim_end_matches('?').to_string(),
        category: category_for(lib).into(),
        value,
        footprint: field("Footprint").unwrap_or_default(),
        footprints: vec![],
        mpn: known.map(|k| k.mpn.to_string()).or(mpn),
        manufacturer: known.map(|k| k.manufacturer.to_string()),
        lcsc: known.map(|k| k.lcsc.to_string()),
        pins,
        units: drawn,
        show_pin_numbers: !pin_numbers_hidden && !power,
        show_pin_names: !pin_names_hidden && !power,
        virtual_part: power,
        keywords: field("ki_keywords").map(|k| k.split_whitespace().map(str::to_string).collect()).unwrap_or_default(),
    }
}

/// A symbol from its file text (tests, and a file the person drops).
pub fn symbol_from_text(lib: &str, text: &str, name: Option<&str>) -> Result<SymbolDef, String> {
    let top = sexpr::parse(text)?;
    let root = top.iter().find(|e| e.is("kicad_symbol_lib")).ok_or("not a KiCad symbol library")?;
    let all: Vec<&Sexp> = root.find_all("symbol").collect();
    let sym = match name {
        Some(n) => all.iter().find(|s| s.arg_text(0) == Some(n)).copied(),
        None => all.first().copied(),
    }
    .ok_or("no such symbol in the file")?;
    let parent = sym.find("extends").and_then(|e| e.arg_text(0)).and_then(|p| all.iter().find(|s| s.arg_text(0) == Some(p)).copied());
    Ok(symbol_def(lib, sym, parent))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_becomes_plain_text() {
        assert_eq!(plain("V_{IN}"), "VIN");
        assert_eq!(plain("~{RESET}"), "/RESET");
        assert_eq!(plain("GPIO0"), "GPIO0");
    }

    #[test]
    fn dates() {
        assert_eq!(iso_date(0), "1970-01-01");
        assert_eq!(iso_date(1_760_000_000), "2025-10-09");
    }

    #[test]
    fn search_prefers_names() {
        let e = |lib: &str, name: &str, d: &str| Entry { id: format!("{lib}:{name}"), library: lib.into(), name: name.into(), description: d.into(), keywords: String::new(), footprint: String::new(), filters: String::new(), datasheet: String::new(), count: 3, repo: Repo::Footprints };
        let idx = vec![e("Package_TO_SOT_SMD", "SOT-23_Handsoldering", "SOT, 3 Pin"), e("Package_TO_SOT_SMD", "SOT-23", "SOT, 3 Pin"), e("Diode_SMD", "D_SOD-123", "SOD-123 diode, sot-23 alternative")];
        let r = search(&idx, "sot-23", 5);
        assert_eq!(r[0].name, "SOT-23");
        assert_eq!(r.len(), 3);
        assert!(search(&idx, "sot-23 zzz", 5).is_empty());
    }
}
