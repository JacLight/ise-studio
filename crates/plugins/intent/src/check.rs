//! The check gate: one report the UI, the agent and the CLI share.
//! "Not checked" is never reported as "clean".

use crate::{Fix, Requirement, RequirementResult, geometry};
use de_core::{Project, Revision, StableId, Status};
use de_solid::{BoxFeature, MaterialRef};
use serde::Serialize;

/// Above this many parts the all-pairs overlap scan is skipped (and says so).
const OVERLAP_SCAN_LIMIT: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    pub code: &'static str,
    pub severity: Severity,
    pub entities: Vec<StableId>,
    pub message: String,
    pub fix: Option<Fix>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CheckReport {
    /// No errors. Warnings and info do not fail the gate.
    pub ok: bool,
    pub revision: Revision,
    pub requirements_total: usize,
    pub requirements_passing: usize,
    pub findings: Vec<Finding>,
}

pub fn check(project: &Project) -> CheckReport {
    let world = project.world();
    let mut findings = Vec::new();
    let mut total = 0;
    let mut passing = 0;
    let mut parts = Vec::new();

    for id in project.entity_ids() {
        if let Some(req) = project.get::<Requirement>(id) {
            total += 1;
            match (project.get::<RequirementResult>(id), project.status(id)) {
                (_, Some(Status::Blocked(reason))) => findings.push(Finding {
                    code: "REQUIREMENT_BLOCKED",
                    severity: Severity::Error,
                    entities: vec![id],
                    message: format!("“{}” cannot be checked: {reason}", req.name),
                    fix: None,
                }),
                (_, Some(Status::Failed(reason))) => findings.push(Finding {
                    code: "REQUIREMENT_UNCHECKABLE",
                    severity: Severity::Error,
                    entities: vec![id],
                    message: format!("“{}” cannot be checked: {}", req.name, reason.trim_start_matches("intent.evaluate: ")),
                    fix: None,
                }),
                (Some(result), _) if result.passed => passing += 1,
                (Some(result), _) => findings.push(Finding {
                    code: "REQUIREMENT_FAILED",
                    severity: Severity::Error,
                    entities: std::iter::once(id).chain(result.violations.iter().flat_map(|v| v.parts.clone())).collect(),
                    message: format!("“{}” fails: {}", req.name, result.summary),
                    fix: result.fix.clone(),
                }),
                (None, _) => findings.push(Finding {
                    code: "REQUIREMENT_NOT_EVALUATED",
                    severity: Severity::Error,
                    entities: vec![id],
                    message: format!("“{}” has not been evaluated", req.name),
                    fix: None,
                }),
            }
        } else if project.get::<BoxFeature>(id).is_some()
            // Components with no placement of their own are templates, checked where they are placed.
            || (project.get::<de_shape::ShapeProgram>(id).is_some() && project.get::<de_solid::Placement>(id).is_some())
        {
            match geometry::part(world, id) {
                Ok(p) => {
                    // Assemblies may give every part its own material instead of one for the whole.
                    if project.get::<MaterialRef>(id).is_none() && p.mass_kg.is_none() {
                        findings.push(Finding {
                            code: "NO_MATERIAL",
                            severity: Severity::Info,
                            entities: vec![id],
                            message: format!("{} has no material, so its mass is unknown", geometry::name(&p)),
                            fix: None,
                        });
                    }
                    parts.push(p);
                }
                Err(e) => findings.push(Finding { code: "PART_INVALID", severity: Severity::Error, entities: vec![id], message: e, fix: None }),
            }
            if let Some(Status::Failed(reason) | Status::Blocked(reason)) = project.status(id) {
                findings.push(Finding { code: "PART_NOT_CURRENT", severity: Severity::Error, entities: vec![id], message: reason.clone(), fix: None });
            }
        }
    }

    if parts.len() > OVERLAP_SCAN_LIMIT {
        findings.push(Finding {
            code: "OVERLAP_SCAN_SKIPPED",
            severity: Severity::Info,
            entities: vec![],
            message: format!("Overlap scan skipped: {} parts exceed the {OVERLAP_SCAN_LIMIT}-part limit", parts.len()),
            fix: None,
        });
    } else {
        for (i, a) in parts.iter().enumerate() {
            for b in &parts[i + 1..] {
                if let (Some((volume, _)), _) = geometry::contact(a, b, 0.0, geometry::check_tolerance(0.0)) {
                    let fix = (a.is_box() && b.is_box()).then(|| {
                        let (axis, delta) = a.aabb.push_apart(&b.aabb, 0.0);
                        crate::move_fix(b, axis, delta, "to clear the overlap")
                    });
                    findings.push(Finding {
                        code: "PARTS_OVERLAP",
                        severity: Severity::Warning,
                        entities: vec![a.id, b.id],
                        message: format!("{} and {} overlap by {} mm³", geometry::name(a), geometry::name(b), geometry::fmt(volume)),
                        fix,
                    });
                }
            }
        }
    }

    findings.sort_by_key(|f| f.severity as u8);
    CheckReport {
        ok: !findings.iter().any(|f| f.severity == Severity::Error),
        revision: project.revision(),
        requirements_total: total,
        requirements_passing: passing,
        findings,
    }
}
