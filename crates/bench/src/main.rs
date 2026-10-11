//! Engine performance workloads. Run with:
//! `cargo run --release -p de-bench -- [part_count]` (default 1,000,000).

use de_core::{
    Actor, Command, CommandError, Envelope, Length, Plugin, Project, Registry, SCHEMA_VERSION, StableId, Tx, Value,
    json, schemars,
};
use de_solid::{BoxFeature, MassProperties, MaterialRef, SolidPlugin};
use serde::Deserialize;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// `bench.boxes.create_many`: N boxes sharing one material, in one transaction.
#[derive(Deserialize, schemars::JsonSchema)]
struct CreateMany {
    count: usize,
    material: StableId,
}

impl Command for CreateMany {
    const NAME: &'static str = "bench.boxes.create_many";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut first = None;
        for i in 0..self.count {
            let id = tx.create();
            first.get_or_insert(id);
            let size = Length::mm(10.0 + (i % 100) as f64);
            tx.set(id, BoxFeature { width: size, depth: size, height: Length::mm(5.0) })?;
            tx.set(id, MaterialRef(self.material))?;
            tx.add_dependency(id, self.material)?;
        }
        Ok(json!({ "first": first }))
    }
}

struct BenchPlugin;

impl Plugin for BenchPlugin {
    fn name(&self) -> &'static str {
        "bench"
    }

    fn register(&self, r: &mut Registry) {
        r.command::<CreateMany>();
    }
}

fn actor() -> Actor {
    Actor { user_id: "bench".into(), session_id: "bench".into() }
}

fn envelope(p: &Project, operation: &str, parameters: Value) -> Envelope {
    Envelope {
        schema_version: SCHEMA_VERSION,
        command_id: StableId::new().to_string(),
        project_id: p.id().into(),
        expected_revision: p.revision(),
        operation: operation.into(),
        parameters,
        actor: actor(),
    }
}

fn commit(p: &mut Project, operation: &str, parameters: Value) -> Value {
    let e = envelope(p, operation, parameters);
    p.commit(e).unwrap_or_else(|err| panic!("{operation}: {err}")).output
}

fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let out = f();
    (out, start.elapsed())
}

fn percentile(samples: &mut [Duration], p: f64) -> Duration {
    samples.sort_unstable();
    samples[((samples.len() as f64 - 1.0) * p).round() as usize]
}

fn rss_mb() -> Option<f64> {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    String::from_utf8(out.stdout).ok()?.trim().parse::<f64>().ok().map(|kb| kb / 1024.0)
}

fn row(name: &str, value: String) {
    println!("{name:<52} {value}");
}

fn main() {
    let parts: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(1_000_000);
    let registry = Arc::new(Registry::with_plugins(&[&SolidPlugin, &BenchPlugin]));
    let mut p = Project::new("bench", registry);
    println!("parts: {parts}, threads: {}\n", std::thread::available_parallelism().map_or(1, |n| n.get()));

    let steel = commit(&mut p, "solid.material.create", json!({ "name": "steel", "density": { "value": 7850, "unit": "kg/m3" } }));
    let steel: StableId = serde_json::from_value(steel["id"].clone()).unwrap();

    let (out, t) = timed(|| commit(&mut p, "bench.boxes.create_many", json!({ "count": parts, "material": steel })));
    let part: StableId = serde_json::from_value(out["first"].clone()).unwrap();
    row(&format!("create {parts} parts in one commit (incl. derive)"), format!("{t:.2?}"));
    row("  journal ops in that commit", p.journal().last().unwrap().ops.len().to_string());

    let mut edits = Vec::with_capacity(2000);
    for i in 0..2000 {
        let params = json!({ "target": part, "width": { "value": 20.0 + (i % 50) as f64, "unit": "mm" } });
        let ((), t) = timed(|| {
            commit(&mut p, "solid.box.set_dimensions", params);
        });
        edits.push(t);
    }
    row("single dimension edit, commit + recompute (p50)", format!("{:.1?}", percentile(&mut edits, 0.5)));
    row("single dimension edit, commit + recompute (p95)", format!("{:.1?}", percentile(&mut edits, 0.95)));

    let mut previews = Vec::with_capacity(2000);
    for _ in 0..2000 {
        let e = envelope(&p, "solid.box.set_dimensions", json!({ "target": part, "height": { "value": 9, "unit": "mm" } }));
        let (_, t) = timed(|| p.preview(&e).unwrap());
        previews.push(t);
    }
    row("preview of a dimension edit (p50)", format!("{:.1?}", percentile(&mut previews, 0.5)));

    let (_, t) = timed(|| commit(&mut p, "solid.material.set_density", json!({ "target": steel, "density": { "value": 2700, "unit": "kg/m3" } })));
    row(&format!("material change -> recompute mass of {parts} parts"), format!("{t:.2?}"));
    let m = p.get::<MassProperties>(part).unwrap().mass_kg.unwrap();
    assert!((m - p.get::<BoxFeature>(part).map(|b| b.width.canonical() * b.depth.canonical() * b.height.canonical()).unwrap() * 2700.0).abs() < 1e-12);

    let (_, t) = timed(|| p.undo(&actor()).unwrap());
    row("undo that material change (verify + recompute)", format!("{t:.2?}"));

    let wire = p.journal().last().unwrap().to_wire();
    let (bytes, t) = timed(|| serde_json::to_vec(&wire).unwrap());
    row("serialize that change set for peers (JSON)", format!("{t:.2?}, {} bytes", bytes.len()));

    if let Some(mb) = rss_mb() {
        row("resident memory", format!("{mb:.0} MB ({:.0} bytes/part incl. journal)", mb * 1048576.0 / parts as f64));
    }
}
