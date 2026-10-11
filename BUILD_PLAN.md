# Unified Design and Engineering Build Plan

Build one shared platform and extend it through complete workflows. Each stage must preserve the project contract and command system defined in the [specification](SPEC.md).

## Sequence and gates

| Stage | Deliverables | Exit gate |
| --- | --- | --- |
| D0 Architecture and feasibility | Exact-kernel/constraint spikes, graph schema, units, stable-reference experiments, browser/native viewport benchmarks, independent-engine connection, interchange corpus, license inventory. | One exact part, one semantic building element, and one rigged mesh coexist in a saved project; viewport results establish where a native client is needed. |
| D1 Shared foundation | Independent Rust engine, React/TypeScript browser interface, local launcher, project graph, commands, recovery, viewport, agent tools, automatic Git history, peer-to-peer sharing, shared views, and optional S3/Spaces/Git destinations. | Two browser clients join one project and edit together without mandatory full downloads; UI and agent share commands and valid history. |
| D2 First complete design workflow | Sketch solver, core solid operations, direct edits, dimensions, simple assemblies, associative drawing, STEP/PDF export, materials, basic render. | Create and edit the spanner end to end; one dimension change updates or invalidates every dependent result correctly. |
| D3 Professional mechanical and drafting | Advanced features, sheets/blocks, configurations, sheet metal, frames/routes, large assemblies, production documentation, compatibility adapters. | Enclosure/frame/assembly and multi-sheet drafting suites pass accuracy and interchange gates. |
| D4 Freeform and procedural authoring | NURBS/SubD, mesh modeling, sculpt/retopology, UVs, procedural graphs, point-cloud workflows, improved materials/renderer. | Freeform product workflow retains both editable authoring and measured manufacturing representations. |
| D5 Building workflows | Architectural types/instances, structural/MEP systems, schedules, analytical links, coordination, IFC/BCF, phases/options and linked sites. | A multi-storey building change propagates through geometry, schedules, documents and coordination. |
| D6 Manufacturing, electronics and analysis | Verified solver adapters, CAM, machine/post verification, additive/fabrication outputs, board/schematic coordination and release packages. | Enclosure/PCB and machined-part workflows pass solver, interference, fabrication and postprocessor gates. |
| D7 Animation and production visualization | Rigging, skinning, curves/clips, retargeting, effects/groom, caches, advanced lighting/shading, passes, scene exchange and render queues. | A production character and product/building animation are fully editable and reproducible from the shared project. |
| D8 Team, platform and replacement hardening | Larger-team collaboration, enterprise policies, tablet authoring, Linux engine packaging, native-file migration routes, performance and release coverage. | Published platform/format/workflow matrices accurately describe tested support; no material silent corruption or false-validity gaps remain. |

Architecture tests for BIM, manufacturing, and animation begin in D0/D1 even though complete workspaces ship later. The shared foundation must handle these entity families before large volumes of mechanical-only code accumulate.

Peer-to-peer editing, shared views, automatic history, and optional cloud/repository storage begin in D1. Their product behavior is defined in specification requirements DE-970 through DE-977.

Native viewport delivery follows measured needs from D0 and accompanies the first workflow that requires it. It connects to the existing engine and session; users switch views without project conversion. It is not deferred to the final hardening stage if an earlier workflow needs native performance.

## Proposed repository layout

```text
apps/web/                 Main React and TypeScript browser interface
apps/launcher/            Engine startup and opening the browser
apps/native/              Native viewport/client for demanding workflows
crates/engine/            Independent engine process and client API
crates/project/           Entities, typed quantities and references
crates/commands/          Validation, transactions and command schemas
crates/history/           Revisions, branch/merge and recovery
crates/storage/           Working store and portable packaging
crates/geometry/          Exact geometry adapter and topology mapping
crates/sketch/            Sketch entities and constraint integration
crates/mesh/              Mesh/SubD and representation conversion
crates/scene/             Evaluation, selection and render graph
crates/viewport/          GPU rendering and input integration
crates/jobs/              Scheduling, cancellation and cache manifests
crates/agent/             Project tools, plans and task evaluations
crates/cli/               Headless operations
modules/                  BIM, CAM, analysis, electronics and animation
adapters/                 Format, solver, renderer and machine providers
services/                 Optional team and remote-compute services
fixtures/                 Geometry, BIM, animation and exchange corpus
benchmarks/               Versioned performance and quality workloads
docs/decisions/           Architecture and capability decisions
```

The directories above are the intended scaffold for implementation, not code created by this specification task.

## First implementation backlog

1. Define unit-safe quantities, entity IDs, revision IDs, representation ownership, dependency edges, and serialization contracts.
2. Prototype exact-kernel calls for box, sketch extrusion, Boolean subtraction, fillet, mass properties, tessellation, and STEP round-trip.
3. Test face/edge identity across parameter changes, suppression, and feature reordering before committing to a reference strategy.
4. Benchmark browser and native viewports with picking, transforms, section planes, instancing, and precise engine-side selection queries; verify shared project/view handoff.
5. Persist one project containing an exact part, a building element with parameters, and a mesh with animation channels.
6. Implement one typed parameter edit with prepare/preview/commit, dependency invalidation, undo/redo, and recovery.
7. Drive that command from the UI, CLI, and agent with identical validation results.
8. Add the first constrained sketch-to-solid workflow and an associative drawing.
9. Build the adjustable-spanner acceptance fixture with known dimensions, configurations, and expected derived outputs.
10. Measure performance, geometry validity, and reference stability before expanding the tool catalog.

## Work dependencies

- Stable IDs, units, transactions, and storage precede every production workspace.
- Robust topology references precede dependable drawings, mates, loads, and manufacturing selections.
- Typed representation conversion precedes claims that mesh/sculpt outputs are production solids.
- Job provenance and stale-state handling precede simulation, CAM, effects caches, and remote rendering.
- Local semantic history precedes collaborative merge.
- Interchange adapters ship only after their format-specific round-trip and loss reports exist.

Team size and schedule are estimated after D0 establishes kernel, adapter, and solver coverage. Any parallel implementation later must use these contracts; it must not create independent document models for individual workspaces.
