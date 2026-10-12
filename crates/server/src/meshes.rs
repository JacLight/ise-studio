//! Display meshes sent once, as binary, by content.
//!
//! Entity views name a mesh by a hash of its content (`{"ref": "…",
//! "triangles": n}`) instead of carrying it as JSON numbers; the browser
//! fetches each mesh it does not hold yet from `GET /mesh/<ref>` (or
//! `/view/<token>/mesh/<ref>` on a view-only link) as raw little-endian
//! arrays, and keeps it. A thousand screws of one size are one mesh on the
//! wire and one geometry on the GPU; a change re-sends only the meshes it
//! made; the browser's HTTP cache holds them across reloads (a ref never
//! changes meaning).
//!
//! Wire format: "FM01", then u32 counts of position floats, normal floats
//! and indices, then the positions (f32, metres), normals (f32) and indices
//! (u32).

use axum::{
    extract::Path,
    http::{StatusCode, header},
    response::IntoResponse,
};
use de_solid::DisplayMesh;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

/// Encoded meshes are dropped, oldest first, past this many bytes (a browser
/// that misses one asks for a fresh snapshot, which registers it again).
const BUDGET: usize = 768 << 20;

#[derive(Default)]
struct Store {
    meshes: HashMap<String, Arc<Vec<u8>>>,
    order: VecDeque<String>,
    bytes: usize,
}

fn store() -> &'static Mutex<Store> {
    static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
    STORE.get_or_init(Default::default)
}

/// How an entity view names its mesh.
#[derive(Serialize)]
pub struct MeshRef {
    #[serde(rename = "ref")]
    pub key: String,
    pub triangles: usize,
}

/// A 128-bit content hash (two independent 64-bit lanes), fast enough to run
/// over every mesh of a snapshot.
fn hash(mesh: &DisplayMesh) -> String {
    const K1: u64 = 0x9E37_79B9_7F4A_7C15;
    const K2: u64 = 0xC2B2_AE3D_27D4_EB4F;
    let (mut a, mut b) = (0x243F_6A88_85A3_08D3u64, 0x1319_8A2E_0370_7344u64);
    let mut mix = |w: u32| {
        a = (a ^ w as u64).wrapping_mul(K1).rotate_left(29);
        b = (b ^ (w as u64).rotate_left(17)).wrapping_mul(K2).rotate_left(31);
    };
    for (lane, n) in [(0u32, mesh.positions.len()), (1, mesh.normals.len()), (2, mesh.indices.len())] {
        mix(lane);
        mix(n as u32);
    }
    mesh.positions.iter().for_each(|v| mix(v.to_bits()));
    mesh.normals.iter().for_each(|v| mix(v.to_bits()));
    mesh.indices.iter().for_each(|v| mix(*v));
    let fold = |x: u64| {
        let x = (x ^ (x >> 33)).wrapping_mul(0xFF51_AFD7_ED55_8CCD);
        let x = (x ^ (x >> 33)).wrapping_mul(0xC4CE_B9FE_1A85_EC53);
        x ^ (x >> 33)
    };
    format!("{:016x}{:016x}", fold(a), fold(b))
}

fn encode(mesh: &DisplayMesh) -> Vec<u8> {
    let mut out = Vec::with_capacity(16 + 4 * (mesh.positions.len() + mesh.normals.len() + mesh.indices.len()));
    out.extend_from_slice(b"FM01");
    for n in [mesh.positions.len(), mesh.normals.len(), mesh.indices.len()] {
        out.extend_from_slice(&(n as u32).to_le_bytes());
    }
    mesh.positions.iter().for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
    mesh.normals.iter().for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
    mesh.indices.iter().for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
    out
}

/// Registers a mesh (once per content) and names it.
pub fn reference(mesh: &DisplayMesh) -> MeshRef {
    let key = hash(mesh);
    let triangles = mesh.indices.len() / 3;
    let mut s = store().lock().unwrap();
    if !s.meshes.contains_key(&key) {
        let bytes = Arc::new(encode(mesh));
        s.bytes += bytes.len();
        s.meshes.insert(key.clone(), bytes);
        s.order.push_back(key.clone());
        while s.bytes > BUDGET && s.order.len() > 1 {
            let Some(old) = s.order.pop_front() else { break };
            if let Some(b) = s.meshes.remove(&old) {
                s.bytes -= b.len();
            }
        }
    }
    MeshRef { key, triangles }
}

fn get(key: &str) -> Option<Arc<Vec<u8>>> {
    store().lock().unwrap().meshes.get(key).cloned()
}

/// `GET /mesh/<ref>`: one mesh, binary, cached by the browser for good.
pub async fn fetch(Path(key): Path<String>) -> axum::response::Response {
    respond(&key)
}

/// `GET /view/<token>/mesh/<ref>`: the same, for a view-only link's page.
pub async fn fetch_shared(
    axum::extract::State(server): axum::extract::State<Arc<crate::Server>>,
    Path((token, key)): Path<(String, String)>,
) -> axum::response::Response {
    if server.shares.lock().unwrap().find(&token, crate::peers::now()).is_none() {
        return (StatusCode::NOT_FOUND, "This link was turned off or has expired.").into_response();
    }
    respond(&key)
}

fn respond(key: &str) -> axum::response::Response {
    match get(key) {
        Some(bytes) => (
            [(header::CONTENT_TYPE, "application/octet-stream"), (header::CACHE_CONTROL, "private, max-age=31536000, immutable")],
            bytes.as_ref().clone(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "no such mesh (ask for a fresh snapshot)").into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mesh_is_named_by_its_content_and_encoded_once() {
        let a = DisplayMesh { positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0], normals: vec![], indices: vec![0, 1, 2] };
        let mut b = a.clone();
        let (ra, rb) = (reference(&a), reference(&b));
        assert_eq!(ra.key, rb.key, "the same content, the same ref");
        assert_eq!(ra.triangles, 1);
        b.positions[4] = 0.5;
        assert_ne!(reference(&b).key, ra.key, "different content, a different ref");
        let bytes = get(&ra.key).unwrap();
        assert_eq!(&bytes[..4], b"FM01");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 9);
        assert_eq!(bytes.len(), 16 + 4 * (9 + 3));
        assert_eq!(f32::from_le_bytes(bytes[16 + 12..16 + 16].try_into().unwrap()), 1.0);
    }
}
