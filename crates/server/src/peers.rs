//! Team working without a host. Every member's engine keeps a full copy of the
//! project; engines connect to each other directly and swap change sets. On
//! the same network that is a direct TCP connection to the peer's IP; over
//! the internet it is a WebSocket over HTTP(S), which passes through proxies.
//!
//! - Each project's team (`<project>.team.json`, next to its log) holds a
//!   team key, the people invited (each with their own token, stored hashed)
//!   and the engines met so far, with their addresses.
//! - A newcomer's engine presents its invite token once and receives the team
//!   key; from then on any two members' engines can sync, whoever invited whom.
//! - On connecting, each side says what it has (a version vector) and sends
//!   what the other lacks; after that, new changes stream both ways. Changes
//!   are relayed onward, so a change reaches everyone through anyone.
//! - Merging is the core's: field by field, the latest write wins on every
//!   copy, and clashes are reported (see `de_core::merge`).
//!
//! Only `/peer` listens beyond this machine; the UI and the rest of the API
//! answer to this computer alone.

use crate::{App, Server};
use axum::extract::ws::{Message, WebSocket};
use de_core::{StableId, WireChangeSet, json};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Digest;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{broadcast, mpsc};

/// A person invited to the project.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Member {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub email: String,
    /// "editor" or "viewer" (a viewer's changes are not taken).
    pub role: String,
    /// SHA-256 of their invite token (the token itself is never stored).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token_hash: String,
    /// The engine that joined with the token (set once it is used).
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default)]
    pub invited_by: String,
    #[serde(default)]
    pub created: u64,
}

/// An engine met so far, and where it can be reached.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Peer {
    pub engine: String,
    pub name: String,
    /// `ws://192.168.1.20:7420/peer`, `wss://ise.example.com/peer`, …
    pub addresses: Vec<String>,
    #[serde(default)]
    pub last_seen: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Team {
    /// Shared by members' engines to recognise each other.
    #[serde(default)]
    pub key: String,
    /// What this engine's person is called, shown to the others.
    #[serde(default)]
    pub my_name: String,
    /// An internet address peers can reach this engine at (through a proxy or
    /// an opened port), e.g. `https://ise.example.com`.
    #[serde(default)]
    pub public_url: Option<String>,
    #[serde(default)]
    pub members: Vec<Member>,
    #[serde(default)]
    pub peers: Vec<Peer>,
    /// An invite this engine is joining with, until the team key arrives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub joining: Option<String>,
    /// This engine's own secret: it proves to the others which member this
    /// is (they know only its hash). Never sent anywhere but in a handshake.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub my_token: String,
    /// People taken off the team: their engines are refused, and nothing more
    /// from them is taken. Spreads to every member like other team news.
    #[serde(default)]
    pub removed: Vec<Removed>,
}

/// Someone taken off the team, and by whom.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Removed {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default)]
    pub token_hash: String,
    #[serde(default)]
    pub by: String,
    #[serde(default)]
    pub at: u64,
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn secret() -> String {
    let mut b = [0u8; 24];
    getrandom::fill(&mut b).expect("the system's random source");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn hash_bytes(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Asks the teammates online for every imported file the project reads that
/// this computer lacks (at most every 20 s per file).
pub fn want_assets(app: &App) {
    let missing: Vec<String> = {
        let project = app.project.lock().unwrap();
        let mut shas = Vec::new();
        for id in project.entity_ids() {
            if let Some(p) = project.get::<de_shape::ShapeProgram>(id) {
                for sha in de_shape::assets::referenced(&p.source) {
                    if de_shape::assets::path(&sha).is_err() && !shas.contains(&sha) {
                        shas.push(sha);
                    }
                }
            }
            // A canvas's picture, too.
            if let Some(c) = project.get::<de_shape::canvas::Canvas>(id)
                && de_shape::assets::path(&c.image).is_err()
                && !shas.contains(&c.image)
            {
                shas.push(c.image.clone());
            }
        }
        shas
    };
    if missing.is_empty() {
        return;
    }
    let links: Vec<mpsc::UnboundedSender<String>> = app.links.lock().unwrap().online.values().map(|o| o.tx.clone()).collect();
    if links.is_empty() {
        return;
    }
    let mut assets = app.assets.lock().unwrap();
    for sha in missing {
        if assets.asked.get(&sha).is_some_and(|t| t.elapsed() < Duration::from_secs(20)) || assets.incoming.contains_key(&sha) {
            continue;
        }
        assets.asked.insert(sha.clone(), std::time::Instant::now());
        for tx in &links {
            let _ = tx.send(serde_json::to_string(&Wire::AssetWanted { sha: sha.clone() }).unwrap());
        }
    }
}

fn hash(token: &str) -> String {
    sha2::Sha256::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

impl Team {
    pub fn path(dir: &std::path::Path, project: &str) -> PathBuf {
        dir.join(format!("{project}.team.json"))
    }

    pub fn load(dir: &std::path::Path, project: &str) -> Team {
        std::fs::read_to_string(Self::path(dir, project)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: &std::path::Path, project: &str) -> std::io::Result<()> {
        std::fs::write(Self::path(dir, project), serde_json::to_string_pretty(self).unwrap())
    }

    /// The team key, made the first time anyone is invited.
    fn ensure_key(&mut self) {
        if self.key.is_empty() {
            self.key = secret();
        }
    }

    /// Learns (or refreshes) an engine and its addresses.
    fn meet(&mut self, engine: &str, name: &str, addresses: &[String]) {
        self.know(engine, name, addresses);
        if let Some(p) = self.peers.iter_mut().find(|p| p.engine == engine) {
            p.last_seen = now();
        }
    }

    /// Learns of an engine without having met it (heard from another member).
    fn know(&mut self, engine: &str, name: &str, addresses: &[String]) {
        match self.peers.iter_mut().find(|p| p.engine == engine) {
            Some(p) => {
                if !name.is_empty() {
                    p.name = name.to_string();
                }
                for a in addresses {
                    if !p.addresses.contains(a) {
                        p.addresses.push(a.clone());
                    }
                }
            }
            None => self.peers.push(Peer { engine: engine.into(), name: name.into(), addresses: addresses.to_vec(), last_seen: now() }),
        }
    }

    /// Whether changes made on `engine` are taken (viewers' and removed people's are not).
    fn may_edit(&self, engine: &str) -> bool {
        !self.is_removed(engine) && !self.members.iter().any(|m| m.engine.as_deref() == Some(engine) && m.role == "viewer")
    }

    pub fn is_removed(&self, engine: &str) -> bool {
        self.removed.iter().any(|r| r.engine.as_deref() == Some(engine))
    }

    /// Gives this engine its own credential if it has none yet (teams made
    /// before credentials: the first one it presents is the one kept).
    pub fn ensure_credential(&mut self, me: &str) {
        if self.my_token.is_empty() {
            self.my_token = secret();
        }
        let h = hash(&self.my_token);
        if let Some(m) = self.members.iter_mut().find(|m| m.engine.as_deref() == Some(me)) {
            if m.token_hash.is_empty() {
                m.token_hash = h;
            }
        }
    }

    /// Whether `engine`, showing `credential`, is the member it claims to be
    /// (and not someone removed). Binds the engine to its member the first
    /// time a member's credential shows up from it.
    fn admit(&mut self, engine: &str, credential: Option<&str>) -> bool {
        if self.is_removed(engine) {
            return false;
        }
        let h = credential.map(hash);
        if h.as_ref().is_some_and(|h| self.removed.iter().any(|r| &r.token_hash == h)) {
            return false;
        }
        match self.members.iter_mut().find(|m| m.engine.as_deref() == Some(engine)) {
            Some(m) if m.token_hash.is_empty() => {
                // An older member without a credential yet: trust on first use.
                if let Some(h) = h {
                    m.token_hash = h;
                }
                true
            }
            Some(m) => h.as_deref() == Some(m.token_hash.as_str()),
            None => match self.members.iter_mut().find(|m| m.engine.is_none() && !m.token_hash.is_empty() && h.as_deref() == Some(m.token_hash.as_str())) {
                Some(m) => {
                    m.engine = Some(engine.to_string());
                    true
                }
                None => false,
            },
        }
    }

    /// Takes someone off the team (by member id); `me` is this engine.
    pub fn remove(&mut self, id: &str, me: &str) -> Result<Removed, String> {
        if self.members.iter().any(|m| m.engine.as_deref() == Some(me) && m.role == "viewer") {
            return Err("viewers cannot remove people".into());
        }
        let m = self.members.iter().find(|m| m.id == id).cloned().ok_or("no such member")?;
        if m.engine.as_deref() == Some(me) {
            return Err("you cannot remove yourself".into());
        }
        let by = if self.my_name.is_empty() { whoami() } else { self.my_name.clone() };
        let r = Removed { id: m.id.clone(), name: m.name.clone(), engine: m.engine.clone(), token_hash: m.token_hash.clone(), by, at: now() };
        self.removed.push(r.clone());
        self.forget_removed();
        Ok(r)
    }

    fn forget_removed(&mut self) {
        let ids: BTreeSet<String> = self.removed.iter().map(|r| r.id.clone()).collect();
        let engines: BTreeSet<String> = self.removed.iter().filter_map(|r| r.engine.clone()).collect();
        self.members.retain(|m| !ids.contains(&m.id));
        self.peers.retain(|p| !engines.contains(&p.engine));
    }

    /// What other engines may know: members without token hashes, and peers.
    fn public(&self) -> Value {
        let members: Vec<Value> = self
            .members
            .iter()
            .map(|m| json!({ "id": m.id, "name": m.name, "email": m.email, "role": m.role, "engine": m.engine, "invited_by": m.invited_by, "created": m.created, "token_hash": m.token_hash }))
            .collect();
        let peers: Vec<Value> = self.peers.iter().filter(|p| !p.engine.is_empty()).map(|p| json!({ "engine": p.engine, "name": p.name, "addresses": p.addresses })).collect();
        json!({ "members": members, "peers": peers, "removed": self.removed })
    }

    /// Takes in what another member's engine knows about the team.
    fn learn(&mut self, public: &Value, me: &str) {
        if let Some(list) = public["removed"].as_array() {
            for r in list {
                if let Ok(r) = serde_json::from_value::<Removed>(r.clone())
                    && !self.removed.iter().any(|x| x.id == r.id)
                {
                    self.removed.push(r);
                }
            }
        }
        if let Some(list) = public["members"].as_array() {
            for m in list {
                let Ok(mut m) = serde_json::from_value::<Member>(m.clone()) else { continue };
                if self.removed.iter().any(|r| r.id == m.id) {
                    continue;
                }
                match self.members.iter_mut().find(|x| x.id == m.id) {
                    Some(x) => {
                        if x.engine.is_none() {
                            x.engine = m.engine.take();
                        }
                        if x.token_hash.is_empty() {
                            x.token_hash = std::mem::take(&mut m.token_hash);
                        }
                    }
                    None => self.members.push(m),
                }
            }
        }
        self.forget_removed();
        if let Some(list) = public["peers"].as_array() {
            for p in list {
                if let Ok(p) = serde_json::from_value::<Peer>(p.clone())
                    && p.engine != me
                    && !p.engine.is_empty()
                {
                    self.know(&p.engine, &p.name, &p.addresses);
                }
            }
        }
    }
}

/// The first thing each side of a peer connection says.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Wire {
    Hello {
        project: String,
        engine: String,
        name: String,
        addresses: Vec<String>,
        known: BTreeMap<String, u64>,
        #[serde(default)]
        key: Option<String>,
        #[serde(default)]
        invite: Option<String>,
        /// The member's own secret (see `Team::my_token`).
        #[serde(default)]
        credential: Option<String>,
    },
    Welcome {
        engine: String,
        name: String,
        addresses: Vec<String>,
        known: BTreeMap<String, u64>,
        /// Given to a newcomer who joined with an invite.
        #[serde(default)]
        key: Option<String>,
        team: Value,
        #[serde(default)]
        credential: Option<String>,
    },
    Changes {
        changes: Vec<WireChangeSet>,
    },
    Team {
        team: Value,
    },
    /// "Do you have this imported file?" (by content hash)
    AssetWanted {
        sha: String,
    },
    /// One piece of an imported file (base64).
    AssetChunk {
        sha: String,
        index: usize,
        total: usize,
        data: String,
    },
    Denied {
        reason: String,
    },
    /// Where this member is pointing and what they have selected (live only).
    Cursor {
        #[serde(default)]
        selected: Option<String>,
        #[serde(default)]
        part: Option<String>,
        #[serde(default)]
        pointer: Option<[f64; 3]>,
    },
}

/// Passes this person's cursor to every teammate online.
pub fn cursor(app: &App, selected: Option<String>, part: Option<String>, pointer: Option<[f64; 3]>) {
    let text = serde_json::to_string(&Wire::Cursor { selected, part, pointer }).unwrap();
    for o in app.links.lock().unwrap().online.values() {
        let _ = o.tx.send(text.clone());
    }
}

/// Live peer connections of one project, by engine.
#[derive(Default)]
pub struct Links {
    pub online: BTreeMap<String, Online>,
    /// When each engine was last connected (or this engine started), to let
    /// the engine with the smaller name dial first and the other only fall back.
    pub last: BTreeMap<String, std::time::Instant>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Online {
    pub name: String,
    pub address: String,
    /// "incoming" or "outgoing".
    pub direction: &'static str,
    pub since: u64,
    /// Sends a message on this connection.
    #[serde(skip)]
    pub tx: mpsc::UnboundedSender<String>,
}

/// Imported files coming in from teammates, and when each was last asked for.
#[derive(Default)]
pub struct Assets {
    pub incoming: BTreeMap<String, Vec<Option<Vec<u8>>>>,
    pub asked: BTreeMap<String, std::time::Instant>,
}

/// Imported files are sent in pieces of this size (well under message limits).
const CHUNK: usize = 2 * 1024 * 1024;

/// Addresses this engine can be reached at: each local network address, and
/// the internet address if one is set.
pub fn my_addresses(port: u16, team: &Team) -> Vec<String> {
    let mut out = Vec::new();
    for ip in lan_ips() {
        out.push(format!("ws://{ip}:{port}/peer"));
    }
    if let Some(url) = team.public_url.as_ref().filter(|u| !u.trim().is_empty()) {
        let u = url.trim().trim_end_matches('/');
        let ws = u.replacen("https://", "wss://", 1).replacen("http://", "ws://", 1);
        out.push(if ws.ends_with("/peer") { ws } else { format!("{ws}/peer") });
    }
    out
}

/// This machine's network addresses (the one the default route uses, first).
pub(crate) fn lan_ips() -> Vec<String> {
    let mut ips = Vec::new();
    // Connecting a UDP socket sends nothing; it only picks the outgoing interface.
    if let Ok(s) = std::net::UdpSocket::bind("0.0.0.0:0")
        && s.connect("192.0.2.1:9").is_ok()
        && let Ok(a) = s.local_addr()
        && !a.ip().is_loopback()
        && !a.ip().is_unspecified()
    {
        ips.push(a.ip().to_string());
    }
    ips
}

/// An invite: which project, whose token, and where the inviter can be reached.
#[derive(Debug, Serialize, Deserialize)]
pub struct Invite {
    pub v: u32,
    /// Project name.
    pub p: String,
    /// The invitee's own token.
    pub t: String,
    /// The inviter's peer addresses.
    pub a: Vec<String>,
    /// Who invited them, and who they are.
    pub by: String,
    pub to: String,
}

impl Invite {
    pub fn encode(&self) -> String {
        use base64::Engine;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).unwrap())
    }

    pub fn decode(text: &str) -> Result<Invite, String> {
        use base64::Engine;
        let text = text.trim();
        let blob = text.rsplit(['=', '#']).next().unwrap_or(text);
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(blob.trim()).map_err(|_| "that is not a ISE Studio invite".to_string())?;
        serde_json::from_slice(&bytes).map_err(|_| "that is not a ISE Studio invite".to_string())
    }
}

/// Creates an invite for one person: their own token, and a link that opens
/// on their computer's Iṣẹ́ Studio (which then joins from here).
pub fn invite(server: &Server, project: &str, app: &App, name: &str, email: &str, role: &str) -> Result<Value, String> {
    if name.trim().is_empty() {
        return Err("who is the invite for? (a name)".into());
    }
    let role = if role == "viewer" { "viewer" } else { "editor" };
    let token = secret();
    let mut team = app.team.lock().unwrap();
    team.ensure_key();
    if team.my_name.is_empty() {
        team.my_name = whoami();
    }
    if team.my_token.is_empty() {
        team.my_token = secret();
    }
    // The inviter is a member too, so others know whose engine this is.
    if !team.members.iter().any(|m| m.engine.as_deref() == Some(server.engine.as_str())) {
        let me = team.my_name.clone();
        let token_hash = hash(&team.my_token);
        team.members.push(Member { id: StableId::new().to_string(), name: me, email: String::new(), role: "editor".into(), token_hash, engine: Some(server.engine.clone()), invited_by: String::new(), created: now() });
    }
    team.ensure_credential(&server.engine);
    let by = team.my_name.clone();
    team.members.push(Member { id: StableId::new().to_string(), name: name.trim().into(), email: email.trim().into(), role: role.into(), token_hash: hash(&token), engine: None, invited_by: by.clone(), created: now() });
    team.save(&server.dir, project).map_err(|e| e.to_string())?;
    let addresses = my_addresses(server.port, &team);
    if addresses.is_empty() {
        return Err("this computer has no network address to invite from".into());
    }
    let code = Invite { v: 1, p: project.into(), t: token, a: addresses.clone(), by: by.clone(), to: name.trim().into() }.encode();
    let link = format!("http://localhost:7420/join?invite={code}");
    Ok(json!({ "link": link, "code": code, "addresses": addresses, "by": by, "role": role }))
}

fn whoami() -> String {
    std::env::var("USER").ok().filter(|u| !u.is_empty()).map(|u| {
        let mut c = u.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or(u)
    }).unwrap_or_else(|| "Me".into())
}

/// Starts joining a project from an invite: remembers the inviter and keeps
/// connecting until the team key and the project have arrived.
pub fn join(server: &Arc<Server>, invite: &str) -> Result<Value, String> {
    let inv = Invite::decode(invite)?;
    let app = server.project(Some(&inv.p))?;
    {
        let mut team = app.team.lock().unwrap();
        if team.key.is_empty() {
            team.joining = Some(inv.t.clone());
        }
        if team.my_name.is_empty() {
            team.my_name = inv.to.clone();
        }
        team.meet("", &inv.by, &inv.a);
        team.save(&server.dir, &inv.p).map_err(|e| e.to_string())?;
    }
    start(server.clone(), inv.p.clone(), app);
    Ok(json!({ "project": inv.p, "from": inv.by }))
}

/// Keeps this engine connected to every known peer of a project.
pub fn start(server: Arc<Server>, project: String, app: Arc<App>) {
    if app.dialing.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    {
        let mut team = app.team.lock().unwrap();
        if !team.key.is_empty() && team.my_token.is_empty() {
            team.ensure_credential(&server.engine);
            let _ = team.save(&server.dir, &project);
        }
    }
    let started = std::time::Instant::now();
    tokio::spawn(async move {
        loop {
            let (targets, have_key) = {
                let team = app.team.lock().unwrap();
                let online = app.links.lock().unwrap();
                // Two engines dialing each other at once would drop both links: the
                // smaller name dials at once, the larger only if nothing came for a while
                // (it may be the only one of the two that can reach the other).
                let patient = |engine: &str| {
                    engine.is_empty() || server.engine.as_str() < engine || online.last.get(engine).copied().unwrap_or(started).elapsed() > Duration::from_secs(6)
                };
                let targets: Vec<(String, Vec<String>)> = team
                    .peers
                    .iter()
                    .filter(|p| p.engine != server.engine && !online.online.contains_key(&p.engine) && patient(&p.engine))
                    .map(|p| (p.engine.clone(), p.addresses.clone()))
                    .collect();
                (targets, !team.key.is_empty() || team.joining.is_some())
            };
            if have_key {
                for (_, addresses) in targets {
                    // One attempt per address; the first that answers wins.
                    for address in addresses {
                        if dial(server.clone(), project.clone(), app.clone(), address).await {
                            break;
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(1500)).await;
        }
    });
}

/// Connects out to a peer address and runs the session. True if it connected.
async fn dial(server: Arc<Server>, project: String, app: Arc<App>, address: String) -> bool {
    let connect = tokio::time::timeout(Duration::from_secs(4), tokio_tungstenite::connect_async(address.as_str())).await;
    let Ok(Ok((socket, _))) = connect else { return false };
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        while let Some(t) = rx.recv().await {
            if sink.send(tokio_tungstenite::tungstenite::Message::Text(t.into())).await.is_err() {
                break;
            }
        }
    });
    let hello = {
        let team = app.team.lock().unwrap();
        let known = app.project.lock().unwrap().known().clone();
        Wire::Hello {
            project: project.clone(),
            engine: server.engine.clone(),
            name: team.my_name.clone(),
            addresses: my_addresses(server.port, &team),
            known,
            key: (!team.key.is_empty()).then(|| team.key.clone()),
            invite: if team.key.is_empty() { team.joining.clone() } else { None },
            credential: (!team.key.is_empty() && !team.my_token.is_empty()).then(|| team.my_token.clone()),
        }
    };
    let _ = tx.send(serde_json::to_string(&hello).unwrap());
    // The first reply must be a welcome.
    let first = loop {
        match stream.next().await {
            Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t))) => break serde_json::from_str::<Wire>(t.as_str()).ok(),
            Some(Ok(_)) => continue,
            _ => break None,
        }
    };
    let Some(Wire::Welcome { engine, name, addresses, known, key, team: team_info, credential }) = first else {
        if let Some(Wire::Denied { reason }) = first {
            eprintln!("peer {address} refused {project}: {reason}");
        }
        writer.abort();
        return true; // it answered: no point trying its other addresses now
    };
    {
        let mut team = app.team.lock().unwrap();
        // A member checks it reached a member (not someone removed) before sending anything.
        if key.is_none() && !team.key.is_empty() && !team.admit(&engine, credential.as_deref()) {
            let _ = team.save(&server.dir, &project);
            drop(team);
            eprintln!("peer {address} for {project} is not a member any more: not syncing with it");
            writer.abort();
            return true;
        }
        if let Some(k) = key {
            team.key = k;
            // The invite token is this member's credential from now on.
            if let Some(t) = team.joining.take() {
                team.my_token = t;
            }
        }
        team.peers.retain(|p| !p.engine.is_empty() || !p.addresses.contains(&address));
        team.meet(&engine, &name, &addresses);
        team.learn(&team_info, &server.engine);
        let _ = team.save(&server.dir, &project);
    }
    let lines = Box::pin(stream.filter_map(|m| async move {
        match m {
            Ok(tokio_tungstenite::tungstenite::Message::Text(t)) => Some(t.to_string()),
            _ => None,
        }
    }));
    run(server, project, app, engine, name, address, "outgoing", known, tx, lines).await;
    writer.abort();
    true
}

/// An engine connecting in (`/peer`): checks who it is, then runs the session.
pub async fn accept(server: Arc<Server>, socket: WebSocket, remote: String) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        while let Some(t) = rx.recv().await {
            if sink.send(Message::Text(t.into())).await.is_err() {
                break;
            }
        }
    });
    let first = loop {
        match stream.next().await {
            Some(Ok(Message::Text(t))) => break serde_json::from_str::<Wire>(t.as_str()).ok(),
            Some(Ok(_)) => continue,
            _ => break None,
        }
    };
    let deny = |tx: &mpsc::UnboundedSender<String>, reason: &str| {
        let _ = tx.send(serde_json::to_string(&Wire::Denied { reason: reason.into() }).unwrap());
    };
    let Some(Wire::Hello { project, engine, name, addresses, known, key, invite, credential }) = first else {
        writer.abort();
        return;
    };
    let Ok(app) = server.project(Some(&project)) else {
        deny(&tx, "no such project here");
        tokio::time::sleep(Duration::from_millis(200)).await;
        writer.abort();
        return;
    };
    // A member's engine (team key), or a newcomer with their own unused invite.
    let verdict: Result<Option<String>, &str> = {
        let mut team = app.team.lock().unwrap();
        let member = !team.key.is_empty() && key.as_deref() == Some(team.key.as_str());
        let verdict = if member {
            if team.admit(&engine, credential.as_deref()) { Ok(None) } else { Err("not on this project's team (removed, or not recognised)") }
        } else if team.is_removed(&engine) {
            Err("removed from this project's team")
        } else {
            let h = invite.as_deref().map(hash);
            let key = team.key.clone();
            match team.members.iter_mut().find(|m| !m.token_hash.is_empty() && h.as_deref() == Some(m.token_hash.as_str())) {
                Some(m) if m.engine.is_none() || m.engine.as_deref() == Some(engine.as_str()) => {
                    m.engine = Some(engine.clone());
                    Ok(Some(key))
                }
                Some(_) => Err("this invite was already used on another computer"),
                None => Err("not a member of this project's team"),
            }
        };
        if verdict.is_ok() {
            team.meet(&engine, &name, &addresses);
            let _ = team.save(&server.dir, &project);
        }
        verdict
    };
    let welcome_key = match verdict {
        Ok(k) => k,
        Err(reason) => {
            deny(&tx, reason);
            tokio::time::sleep(Duration::from_millis(200)).await;
            writer.abort();
            return;
        }
    };
    let welcome = {
        let team = app.team.lock().unwrap();
        Wire::Welcome {
            engine: server.engine.clone(),
            name: team.my_name.clone(),
            addresses: my_addresses(server.port, &team),
            known: app.project.lock().unwrap().known().clone(),
            key: welcome_key,
            team: team.public(),
            credential: (!team.my_token.is_empty()).then(|| team.my_token.clone()),
        }
    };
    let _ = tx.send(serde_json::to_string(&welcome).unwrap());
    start(server.clone(), project.clone(), app.clone());
    let lines = Box::pin(stream.filter_map(|m| async move {
        match m {
            Ok(Message::Text(t)) => Some(t.to_string()),
            _ => None,
        }
    }));
    run(server, project, app, engine, name, remote, "incoming", known, tx, lines).await;
    writer.abort();
}

/// One live connection to another member's engine, once both sides know each
/// other: send what they lack, then stream changes both ways until it drops.
#[allow(clippy::too_many_arguments)]
async fn run(
    server: Arc<Server>,
    project: String,
    app: Arc<App>,
    engine: String,
    name: String,
    address: String,
    direction: &'static str,
    their_known: BTreeMap<String, u64>,
    tx: mpsc::UnboundedSender<String>,
    mut lines: std::pin::Pin<Box<dyn futures_util::Stream<Item = String> + Send>>,
) {
    {
        let mut links = app.links.lock().unwrap();
        if links.online.contains_key(&engine) {
            return; // already connected the other way
        }
        links.online.insert(engine.clone(), Online { name: name.clone(), address: address.clone(), direction, since: now(), tx: tx.clone() });
        links.last.insert(engine.clone(), std::time::Instant::now());
    }
    app.team_changed();
    // Subscribe before reading the log so no change falls between.
    let mut shared = app.shared.subscribe();
    let missing: Vec<WireChangeSet> = {
        let project = app.project.lock().unwrap();
        project
            .log()
            .iter()
            .filter(|w| w.stamp.as_ref().is_some_and(|s| their_known.get(&s.origin).copied().unwrap_or(0) < s.clock))
            .cloned()
            .collect()
    };
    for chunk in missing.chunks(500) {
        let _ = tx.send(serde_json::to_string(&Wire::Changes { changes: chunk.to_vec() }).unwrap());
    }
    // Team news (new members, engines and their addresses) spreads by gossip:
    // every few seconds each side tells the other what it knows.
    let gossip_tx = tx.clone();
    let gossip_app = app.clone();
    let gossip = tokio::spawn(async move {
        loop {
            let team = gossip_app.team.lock().unwrap().public();
            if gossip_tx.send(serde_json::to_string(&Wire::Team { team }).unwrap()).is_err() {
                break;
            }
            want_assets(&gossip_app);
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
    let forward_tx = tx.clone();
    let peer = engine.clone();
    let forward = tokio::spawn(async move {
        loop {
            match shared.recv().await {
                Ok(wire) => {
                    // No need to send a change back to the engine it came from.
                    if wire.stamp.as_ref().is_some_and(|s| s.origin == peer) {
                        continue;
                    }
                    if forward_tx.send(serde_json::to_string(&Wire::Changes { changes: vec![(*wire).clone()] }).unwrap()).is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    while let Some(text) = lines.next().await {
        // Someone removed (here, or by another member) is dropped at once.
        if app.team.lock().unwrap().is_removed(&engine) {
            break;
        }
        match serde_json::from_str::<Wire>(&text) {
            Ok(Wire::Changes { changes }) => {
                let may = { app.team.lock().unwrap().clone() };
                let changes: Vec<WireChangeSet> = changes.into_iter().filter(|w| w.stamp.as_ref().is_none_or(|s| may.may_edit(&s.origin))).collect();
                if let Err(e) = app.receive(changes) {
                    eprintln!("{project}: a change from {name} could not be merged: {e}");
                }
                want_assets(&app);
            }
            Ok(Wire::AssetWanted { sha }) => {
                // Send it if we have it (members only reach this point).
                if let Ok(path) = de_shape::assets::path(&sha) {
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let Ok(bytes) = tokio::fs::read(&path).await else { return };
                        use base64::Engine;
                        let total = bytes.len().div_ceil(CHUNK).max(1);
                        for (index, piece) in bytes.chunks(CHUNK).enumerate() {
                            let data = base64::engine::general_purpose::STANDARD.encode(piece);
                            if tx.send(serde_json::to_string(&Wire::AssetChunk { sha: sha.clone(), index, total, data }).unwrap()).is_err() {
                                return;
                            }
                        }
                    });
                }
            }
            Ok(Wire::AssetChunk { sha, index, total, data }) => {
                use base64::Engine;
                let Ok(piece) = base64::engine::general_purpose::STANDARD.decode(data) else { continue };
                let done = {
                    let mut assets = app.assets.lock().unwrap();
                    let slots = assets.incoming.entry(sha.clone()).or_insert_with(|| vec![None; total.clamp(1, 100_000)]);
                    if index < slots.len() {
                        slots[index] = Some(piece);
                    }
                    if slots.iter().all(Option::is_some) { assets.incoming.remove(&sha).map(|s| s.into_iter().flatten().flatten().collect::<Vec<u8>>()) } else { None }
                };
                if let Some(bytes) = done {
                    // Content-addressed: only the right bytes are kept.
                    if hash_bytes(&bytes) == sha {
                        // Kept as what it is (a STEP file, a picture, a font).
                        if de_shape::assets::store_as(&sha, &bytes).is_ok() {
                            app.asset_arrived(&sha);
                        }
                    } else {
                        eprintln!("{project}: an imported file from {name} did not match its hash; asking again");
                        app.assets.lock().unwrap().asked.remove(&sha);
                    }
                }
            }
            Ok(Wire::Cursor { selected, part, pointer }) => {
                let _ = app.events.send(json!({ "type": "cursor", "engine": engine, "name": name, "selected": selected, "part": part, "pointer": pointer }).to_string().into());
            }
            Ok(Wire::Denied { reason }) => {
                eprintln!("{project}: {name} ended the link: {reason}");
                break;
            }
            Ok(Wire::Team { team }) => {
                let changed = {
                    let mut t = app.team.lock().unwrap();
                    let before = serde_json::to_string(&t.public()).unwrap();
                    t.learn(&team, &server.engine);
                    let changed = serde_json::to_string(&t.public()).unwrap() != before;
                    if changed {
                        let _ = t.save(&server.dir, &project);
                    }
                    changed
                };
                if changed {
                    app.team_changed();
                }
            }
            _ => {}
        }
    }
    forward.abort();
    gossip.abort();
    {
        let mut links = app.links.lock().unwrap();
        links.online.remove(&engine);
        links.last.insert(engine.clone(), std::time::Instant::now());
    }
    let _ = app.events.send(json!({ "type": "cursor", "engine": engine, "gone": true }).to_string().into());
    app.team_changed();
}

/// What the team panel shows.
pub fn status(server: &Server, app: &App) -> Value {
    let team = app.team.lock().unwrap();
    let online = app.links.lock().unwrap().online.clone();
    let project = app.project.lock().unwrap();
    let members: Vec<Value> = team
        .members
        .iter()
        .map(|m| {
            let me = m.engine.as_deref() == Some(server.engine.as_str());
            let on = m.engine.as_ref().is_some_and(|e| online.contains_key(e));
            json!({ "id": m.id, "name": m.name, "email": m.email, "role": m.role, "joined": m.engine.is_some(), "me": me, "online": me || on })
        })
        .collect();
    let peers: Vec<Value> = online.iter().map(|(e, o)| json!({ "engine": e, "name": o.name, "address": o.address, "direction": o.direction, "since": o.since })).collect();
    let engines: BTreeSet<&String> = project.known().keys().collect();
    json!({
        "engine": server.engine,
        "my_name": if team.my_name.is_empty() { whoami() } else { team.my_name.clone() },
        "public_url": team.public_url,
        "addresses": my_addresses(server.port, &team),
        "has_team": !team.key.is_empty(),
        "joining": team.joining.is_some(),
        "members": members,
        "online": peers,
        "waiting": project.waiting(),
        "changes": project.log().len(),
        "engines": engines.len(),
        "conflicts": app.conflicts.lock().unwrap().clone(),
        "removed": team.removed.iter().map(|r| json!({ "name": r.name, "by": r.by, "at": r.at })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str, name: &str, token: &str, engine: Option<&str>) -> Member {
        Member { id: id.into(), name: name.into(), email: String::new(), role: "editor".into(), token_hash: hash(token), engine: engine.map(str::to_string), invited_by: "Ada".into(), created: 0 }
    }

    #[test]
    fn members_prove_who_they_are_and_removed_people_are_refused_everywhere() {
        // Ada's engine made the team and invited Bo (token "bo") and Cy (token "cy").
        let mut ada = Team { key: "k".into(), my_name: "Ada".into(), ..Default::default() };
        ada.members.push(Member { engine: Some("e-ada".into()), token_hash: String::new(), ..member("1", "Ada", "", None) });
        ada.ensure_credential("e-ada");
        assert!(!ada.my_token.is_empty() && ada.members[0].token_hash == hash(&ada.my_token), "Ada's engine has its own credential");
        ada.members.push(member("2", "Bo", "bo", None));
        ada.members.push(member("3", "Cy", "cy", None));
        // Bo's engine shows up with Bo's credential: admitted, and bound to Bo.
        assert!(ada.admit("e-bo", Some("bo")));
        assert_eq!(ada.members[1].engine.as_deref(), Some("e-bo"));
        // The wrong credential, or none, is refused; so is an engine nobody knows.
        assert!(!ada.admit("e-bo", Some("cy")) && !ada.admit("e-bo", None));
        assert!(!ada.admit("e-stranger", Some("nope")));
        assert!(ada.admit("e-cy", Some("cy")));

        // Bo's engine learns the team from Ada (gossip): the members and their credential hashes.
        let mut bo = Team { key: "k".into(), my_name: "Bo".into(), my_token: "bo".into(), ..Default::default() };
        bo.learn(&ada.public(), "e-bo");
        assert!(bo.admit("e-cy", Some("cy")), "Bo recognises Cy too");

        // Ada removes Cy. Ada refuses Cy at once, whatever engine Cy claims to be.
        assert!(ada.remove("3", "e-ada").is_ok());
        assert!(ada.members.iter().all(|m| m.id != "3"));
        assert!(!ada.admit("e-cy", Some("cy")));
        assert!(!ada.admit("e-bo", Some("cy")), "Cy's credential is refused even from another engine id");
        assert!(!ada.may_edit("e-cy"), "and changes from Cy's engine are no longer taken");
        assert!(ada.remove("1", "e-ada").is_err(), "nobody removes themselves");

        // The news reaches Bo: Bo refuses Cy too, and cannot be talked into re-adding Cy.
        bo.learn(&ada.public(), "e-bo");
        assert!(!bo.admit("e-cy", Some("cy")));
        assert!(bo.members.iter().all(|m| m.id != "3"));
        let mut stale = ada.public();
        stale["removed"] = json!([]);
        stale["members"].as_array_mut().unwrap().push(json!({ "id": "3", "name": "Cy", "role": "editor", "engine": "e-cy", "token_hash": hash("cy") }));
        bo.learn(&stale, "e-bo");
        assert!(bo.members.iter().all(|m| m.id != "3"), "old news does not bring Cy back");

        // A viewer cannot remove anyone.
        let mut viewer = bo.clone();
        viewer.members.iter_mut().find(|m| m.id == "2").unwrap().role = "viewer".into();
        assert!(viewer.remove("1", "e-bo").is_err());
    }
}
