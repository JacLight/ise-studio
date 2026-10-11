# Unified Design and Engineering Product and Technical Specification

Version: 0.3. Date: October 8, 2026. Status: proposed implementation baseline.

Build an AI-first design and engineering editor with one application, one project model, and one shared engine. The user and agent work on the same editable objects. Drafting, solid modeling, assemblies, building design, manufacturing, mesh modeling, animation, and rendering are workspaces over that project.

A spanner remains the same part when it is dimensioned, assembled, simulated, machined, rendered, or animated. A building component remains the same component when it appears in a floor plan, schedule, coordination view, fabrication drawing, or walkthrough. The product must preserve these relationships without users exporting and reimporting their own work between workspaces.

This is a separate product from the PDF Editor. PDF drawing output may use an integration, but neither application's core functionality depends on running the other.

## 1 Product boundaries and principles

The complete target spans the principal workflows associated with AutoCAD, Revit, Inventor, Fusion, Maya, 3ds Max, SOLIDWORKS, Rhino, and Shapr3D. It is not nine applications packaged behind a launcher. Workspaces change tools and views while retaining the same objects, IDs, revisions, selection, materials, and dependency graph.

The engine is one coherent platform with specialized capabilities: exact geometry, mesh operations, constraints, semantic building data, animation evaluation, simulation, toolpaths, and rendering. Each capability uses the shared model and command system. Exact solids, subdivision meshes, building semantics, and simulation fields have appropriate typed representations inside that model.

Product rules:

- AI can inspect, create, modify, validate, and explain real structured designs through the same commands as direct editing.
- Manual sketching, precise numeric entry, object manipulation, and expert workflows remain first-class.
- Every modification is a transaction with a revision, an inspectable diff, and a recoverable history.
- Exact engineering geometry is retained through visualization. Tessellated render meshes are derived data unless the user explicitly chooses mesh authoring.
- Changes trigger selective recomputation. Dependent outputs either update or clearly show that they are stale, blocked, or invalid.
- Advanced workspaces load on demand. A user sketching a bracket should not need active fluid solvers, character-rig tools, and render workers.
- Fast local interaction, AI-first operation, and peer-to-peer collaboration are foundational. Local work works offline when its required content is available. Cloud storage, a remote Git repository, and remote compute are optional.
- Users share one project, follow the same view, and edit together without requiring a full project download on every connected device. Automatic history tracks changes in the background.
- Rust owns the project graph, commands, scheduling, storage, and application services. Specialized native libraries are allowed behind explicit interfaces.
- Full product scope is retained through staged releases; each release publishes its actual supported workflows.
- No blanket promise of native-file, plugin, solver, or feature-history compatibility is made without testing the specific version and capability.

Working product name: Design Engineering. The main interface runs in a normal browser using React and TypeScript. The independent Rust engine initially supports Windows and macOS, then Linux. An optional native viewport/client handles demanding interactive workloads when browser performance or capabilities are insufficient. Both clients use the same engine and project. Tablet authoring follows the same contracts, with supported browser/OS combinations validated explicitly.

## 2 Replacement coverage

The table maps established product categories to the intended workspaces. Official product references support these broad categories; the detailed requirements below are this product's proposed scope.

| Existing product | Workflows to cover in this application | Official scope reference |
| --- | --- | --- |
| AutoCAD | Precision 2D drafting, drawing management, blocks, annotation, layouts, plotting, and 3D drafting. | [AutoCAD overview](https://www.autodesk.com/products/autocad/overview.htm) |
| Revit | Building objects and systems, architectural/structural/MEP authoring, documentation, schedules, and coordination. | [Revit product details](https://www.autodesk.com/products/revit/product-details) |
| Inventor | Parametric parts, assemblies, production drawings, specialized mechanical design, and configuration. | [Inventor product details](https://www.autodesk.com/products/inventor/product-details) |
| Fusion | Connected design, manufacturing, simulation, electronics, and generative-design workflows. | [Fusion features](https://www.autodesk.com/products/fusion-360/features) |
| Maya | Polygon modeling, character workflows, rigging, animation, procedural effects, and rendering. | [Maya product details](https://www.autodesk.com/products/maya/product-details) |
| 3ds Max | Modeling, modifiers, scene assembly, procedural content, animation, and architectural/product visualization. | [3ds Max modeling](https://www.autodesk.com/solutions/media-entertainment/3d-modeling-with-3ds-max) |
| SOLIDWORKS | Parts, assemblies, associative drawings, mechanical specialization, and production-oriented design. | [SOLIDWORKS Design](https://www.solidworks.com/product/solidworks-design) |
| Rhino | Precision curves and NURBS surfaces, SubD, mesh work, geometry analysis, and procedural design. | [Rhino features](https://www.rhino3d.com/en/features/) |
| Shapr3D | Accessible direct and history-based modeling, stylus/touch interaction, drawings, and visualization. | [Shapr3D modeling space](https://support.shapr3d.com/hc/en-us/articles/7873880676508-Shapr3D-modeling-space) |

Existing proprietary plugin marketplaces and vendor cloud services are migration/integration targets. Replacement claims are assessed by completed user workflows, editable outputs, accuracy, and interoperability, not a count of toolbar buttons.

## 3 User experience

### 3.1 Common shell

The application opens to the 2D/3D canvas with a first-class AI mini composer and minimal floating controls. The assistant has mini, docked, floating, expanded and hidden states sharing one conversation. All controls can hide, including project navigation, tool shelves, inspectors, history, job surfaces and AI. Navigation, properties, dependencies, sketch constraints, building properties, material nodes and animation curves open as requested work surfaces; no discipline imposes permanent panel columns or a timeline on every user. Docking reserves viewport width, while floating and mini modes overlay it. Chat presentation changes preserve the renderer, camera, selection, draft, conversation and pending operations. Provide focus mode, accessible restore controls and command search. See DESIGN.md version 2 for the complete interaction contract.

Workspaces: Sketch and Draft, Model, Surface and Sculpt, Assemble, Build, Document, Simulate, Manufacture, Electronics, Animate, and Render. The animation timeline is visible when useful. Model history and animation time are separate concepts and separate controls.

Switching workspace preserves the project, object selection, undo history, and camera when applicable. Multiple views can show a model, drawing, simulation, or schedule simultaneously. A workspace can be customized and saved without duplicating the design.

### 3.2 Interaction

- Mouse, keyboard, trackpad, stylus, touch, and supported 3D navigation devices.
- Customizable shortcuts and navigation presets; discoverable commands remain consistent across presets.
- Context-aware selection of entities, edges, faces, vertices, features, instances, bones, fields, and building systems.
- Selection filters, cycling through occluded entities, box/lasso selection, isolation, section cuts, and saved sets.
- Numeric input with units and expressions; mixed display units; precision controls independent of actual model tolerance.
- Constraint inference, snapping, alignment guides, manipulator handles, and editable previews before commit.
- Accessible menus, property panels, command search, and nonvisual object descriptions. Document the limitations of nonvisual manipulation of complex geometry.
- Clear labels for physical materials, appearance materials, design geometry, simulation deformation, and animation poses.

### 3.3 Example AI interaction

User: “Make the spanner handle lighter, keep the 30 mm opening, and preserve the mounting dimensions.”

The agent locates the part and governing dimensions, proposes editable changes, preserves the explicit constraints, updates dependent features, runs available geometry checks, and compares mass using the assigned material. If a load study exists, it can rerun that study and report its assumptions. If none exists, it does not claim the new handle is strong enough merely because geometry is valid. The user reviews the part and continues with direct modeling or another request.

## 4 Shared project model

### 4.1 Entity system

| Entity | Purpose and required relationships |
| --- | --- |
| Project and revision | Ownership, units, coordinate systems, schema versions, immutable revisions, branches, and active working state. |
| Design object | Stable identity, name, type, metadata, authoritative representation, materials, variants, and dependencies. |
| Geometry representation | Exact B-rep, curves/surfaces, mesh/SubD, point cloud, volume/field, or 2D entities with provenance. |
| Feature | Operation, parameters, inputs, references, output identities, suppression state, and rebuild diagnostics. |
| Sketch and constraint | Plane/frame, entities, dimensions, equations, solver state, degrees of freedom, and conflict sets. |
| Definition and instance | Reusable part/family/asset definition plus instance transform, overrides, and configuration. |
| Assembly relationship | Joints, mates, limits, contacts, hierarchy, and physical degrees of freedom. |
| Building element | Type/instance parameters, level, phase, classification, spatial containment, host and system connections. |
| Material | Physical properties with units, appearance graph, provenance, and optional manufacturing properties. |
| Drawing and sheet | Source revision, model views, annotations, tolerances, layout, scales, and publication state. |
| Study | Solver configuration, mesh, loads, boundaries, material assumptions, source revision, and results. |
| Manufacturing setup | Stock, fixture, machine, tool library, operations, postprocessor version, and verification status. |
| Animation and rig | Timeline, clips, channels, bones/controllers, constraints, deformation graph, and caches. |
| Render scene | Object references, cameras, lighting, appearance overrides, environment, color settings, and outputs. |
| External reference | Source file/service, version/hash, coordinate mapping, supported semantics, and update policy. |
| Change and issue | Actor, command set, affected entities, discussion anchors, checks, and resolution. |

Objects are not identified by screen names or transient array positions. A reference contains a stable entity ID and, where necessary, a semantic/topological selector. Rebuilding a feature records correspondence between old and new topology. Ambiguous face or edge matches create a repairable reference failure rather than silently binding to the wrong face.

### 4.2 Geometry ownership and conversion

Each object declares its authoritative representation. An exact solid may own a cached display mesh; editing the solid invalidates and rebuilds that mesh. A sculpted mesh can remain authoritative without pretending to be an exact solid. A fitting operation can create a linked NURBS/B-rep result with a measured deviation and an explicit conversion node.

Mesh-to-solid, SubD-to-NURBS, scanned-data reconstruction, and curve approximation record tolerances, error bounds, and source references. Lossy transformations never overwrite the source silently. Workspaces operate on the same object or an explicit derived object, with the relationship visible in the graph.

### 4.3 Units and precision

Use dimensional quantities, not unlabeled floats, at command boundaries. Store length in a declared canonical unit, angles in a declared convention, and transformations with explicit coordinate frames. Preserve input/display units separately. Kernel adapters perform explicit unit conversion and report effective tolerance.

Engineering calculations use double precision or better where required. Large-site georeferencing keeps a precise global frame and local coordinates for interactive rendering. Small-part tolerances and kilometer-scale coordinates must coexist without viewport jitter or invalid geometric comparisons. Every import records source units, axis orientation, handedness, and scale assumptions.

### 4.4 Dependencies and recomputation

Use a directed dependency graph for features and derived artifacts; solver groups handle intentional coupled constraints. An edit invalidates only its transitive dependents. Jobs use immutable input snapshots and publish results only if the revision and parameters still match.

All computed objects expose `current`, `stale`, `computing`, `failed`, or `blocked` state. Drawings can refresh immediately while an expensive simulation waits. Manufacturing release is blocked when required source geometry or verification is stale. Users can inspect the last result, but its source revision is always visible.

Do not promise byte-identical floating-point output across unrelated GPU/solver platforms. Require stable semantic results within declared tolerances, pinned solver versions for reproducibility, and seeds for stochastic generation.

## 5 Workspace requirements

All feature groups below belong to the complete product. Build order is defined separately. Each group requires UI commands, agent access where meaningful, persistence, undo, and acceptance fixtures.

### 5.1 Sketching and 2D drafting

| ID | Required capability | Acceptance workflow |
| --- | --- | --- |
| DE-100 | Lines, polylines, arcs, circles, ellipses, splines, text, hatches, regions, construction geometry, and images/underlays. | Produce a dimensioned multi-layer technical drawing with correct geometry and text. |
| DE-101 | Coincident, parallel, perpendicular, tangent, concentric, equal, symmetric, fixed, distance, angle, and expression constraints. | Detect under/overconstraint and identify a useful conflicting constraint set. |
| DE-102 | Trim, extend, offset, fillet, chamfer, mirror, patterns, arrays, grips, snaps, polar tracking, and custom coordinate systems. | Exact coordinates and expected topology match reference cases. |
| DE-103 | Layers, line styles, line weights, colors, reusable/parameterized blocks, attributes, external references, and object selection queries. | Updating a definition updates its instances while preserving overrides. |
| DE-104 | Model/paper spaces, sheets, scaled viewports, annotation scales, dimension styles, tables, title blocks, page setups, and batch plotting. | Printed scale, line weights, and dimensions remain correct across sheet sizes. |
| DE-105 | Drawing comparison, revision clouds, data extraction, field expressions, sheet sets, and underlay alignment. | A multi-sheet revision can be reviewed and published with traceable sources. |

### 5.2 Solid and direct modeling

| ID | Required capability | Acceptance workflow |
| --- | --- | --- |
| DE-200 | Parametric extrude, revolve, sweep, loft, holes/threads, fillet, chamfer, shell, draft, ribs, patterns, and Boolean operations. | Model and regenerate a realistic machined part through parameter changes. |
| DE-201 | Direct face move/offset/delete/replace, push-pull, local repair, feature recognition, and history-aware direct edits. | Edit an imported solid without losing its exact geometry or corrupting neighboring faces. |
| DE-202 | Feature reordering, suppression, rollback, configurations, equations, design tables, and reusable feature groups. | Variants rebuild deterministically with explicit invalid-configuration errors. |
| DE-203 | Mass, center of gravity, inertia, volume, area, section properties, clearance, draft, and thickness inspection. | Values agree with analytic references and known solids within declared tolerances. |
| DE-204 | Sheet metal with thickness rules, bends, relief, seams, bend allowances, unfold/refold, and flat patterns. | A formed part and its flat pattern remain linked under dimension changes. |
| DE-205 | Weldments, frames, profiles, joints, cut lists, fasteners, standard components, routing, tubing, piping, and cable/harness paths. | A frame/route change updates lengths, connections, and the bill of materials. |
| DE-206 | Mold/tooling features, parting analysis, core/cavity, draft validation, and derived tooling components. | Geometry is editable and interference/draft issues are identified. |

### 5.3 Curves, surfaces, mesh, and sculpting

| ID | Required capability | Acceptance workflow |
| --- | --- | --- |
| DE-300 | NURBS curves/surfaces, control-point editing, rebuild, trim/untrim, join, split, project, intersect, blend, patch, and offsets. | Construct a freeform enclosure with measured continuity and sewing tolerances. |
| DE-301 | Position/tangent/curvature continuity, curvature plots, zebra/reflection analysis, deviation maps, and fairing tools. | Boundary continuity is evaluated numerically, not solely by appearance. |
| DE-302 | Polygon vertices/edges/faces, extrusion, inset, bevel, bridge, loop cuts, normals, smoothing, symmetry, and instancing. | Build and edit a reusable production mesh with stable attributes. |
| DE-303 | Subdivision surfaces, creases, cages, multiresolution sculpting, brushes, masks, remesh, retopology, and decimation. | High-resolution detail can be baked to a lower-resolution asset without silent UV loss. |
| DE-304 | UV seams/unwrapping, packing, UDIM workflows, texture painting, projection, baking, and material assignments. | Texture alignment survives supported edits and interchange. |
| DE-305 | Mesh repair, watertightness, self-intersection checks, scans/point clouds, alignment, section extraction, and surface reconstruction. | Reconstruction reports residual error and preserves the original scan. |
| DE-306 | Typed procedural node graphs, expressions, reusable groups, instances, field-driven geometry, and parameter sweeps. | A procedural result stays editable and can drive exact or mesh operations through typed adapters. |

### 5.4 Assemblies and mechanisms

| ID | Required capability | Acceptance workflow |
| --- | --- | --- |
| DE-400 | Definitions/instances, nested assemblies, in-context design, top-down references, configurations, and lightweight loading. | Reusing a part does not duplicate its geometry; an edit propagates to relevant instances. |
| DE-401 | Fixed, revolute, slider, cylindrical, planar, spherical, screw, gear, and other declared kinematic relationships. | A mechanism respects remaining degrees of freedom and joint limits. |
| DE-402 | Contact, interference, minimum distance, exploded views, assembly sequences, and motion studies. | A collision is tied to objects and time; the result identifies its geometry revision. |
| DE-403 | Bills of materials, part numbers, quantities, purchase/manufacture classification, substitution, and configuration-specific outputs. | BOM quantities and cut lists agree with the active configuration. |
| DE-404 | Assembly variants, virtual components, suppressed parts, external components, packaging, and revision comparison. | A packaged assembly opens with resolved dependencies and clear missing-reference diagnostics. |

### 5.5 Architecture and BIM

| ID | Required capability | Acceptance workflow |
| --- | --- | --- |
| DE-500 | Sites, coordinates, levels, grids, walls, floors, roofs, ceilings, doors, windows, curtain systems, stairs, railings, rooms, and spaces. | Changing a level updates hosted elements and related documentation coherently. |
| DE-501 | Parametric families/types, instances, hosts, connectors, materials/layers, classifications, and reusable libraries. | Changing a type updates instances while retaining valid instance overrides. |
| DE-502 | Structural columns, beams, braces, foundations, slabs, reinforcement, connections, and analytical-model associations. | Physical and analytical models retain explicit mappings and validation errors. |
| DE-503 | MEP ducts, pipes, cable trays, conduits, fixtures, equipment, connectors, systems, slopes, and routing. | Connectivity and sizing inputs survive edits; broken connections are discoverable. |
| DE-504 | Plans, sections, elevations, details, tags, room/area schedules, quantities, material takeoffs, and sheet sets. | A changed wall updates all dependent views and scheduled quantities. |
| DE-505 | Phases, demolition/new work, design options, worksets/ownership, linked models, model federation, clashes, and issue tracking. | Phased views and quantities match filters; linked coordinate systems align. |
| DE-506 | Terrain/topography, building pads, site context, sun/shadow studies, spaces/zones, and energy-analysis inputs. | Geographic and model coordinates remain consistent; results state assumptions. |
| DE-507 | IFC import/export, property mapping, classification dictionaries, BCF issues, and configurable information requirements. | GUIDs, spatial hierarchy, selected property sets, and supported relationships round-trip. |
| DE-508 | Quantities/cost attributes, construction sequencing, fabrication-linked components, and revision packages. | A dated sequence or estimate references the correct configuration and revision. |

BIM objects are typed design objects in the common graph. Their geometry can be modeled, rendered, analyzed, or fabricated in other workspaces while building semantics remain attached. Automated checks identify rule violations; the product does not infer that a model meets every local building requirement from a geometry check.

### 5.6 Drawings and documentation

| ID | Required capability | Acceptance workflow |
| --- | --- | --- |
| DE-600 | Associative orthographic, projected, section, detail, auxiliary, broken, and exploded views. | A source edit updates views or flags a broken reference. |
| DE-601 | Dimensions, tolerances, datums, geometric tolerancing, surface finish, weld symbols, hole/thread notes, balloons, and tables. | Annotations retain their geometric anchors after supported edits. |
| DE-602 | Templates, standards profiles, revisions, change marks, sheet sets, BOM/cut lists, and model-based annotations. | Publish a traceable multi-sheet release from one configuration. |
| DE-603 | PDF, SVG, DXF, validated DWG paths, raster output, and print/plot. | Line widths, fonts, scale, layer mappings, and annotations survive tested exports. |

Manufacturing tolerances, display rounding, geometry tolerance, and solver tolerance are separate properties. Standards profiles must name their version and implemented rule coverage.

### 5.7 Simulation and engineering analysis

| ID | Required capability | Acceptance workflow |
| --- | --- | --- |
| DE-700 | Study setup, materials, loads, supports, contacts, connections, meshing, refinement, and solver controls. | The setup records units, assumptions, and source revision. |
| DE-701 | Linear static, modal, buckling, thermal, and thermomechanical studies. | Results agree with analytic and published benchmark problems within specified tolerances. |
| DE-702 | Nonlinear materials/geometry, contact, transient dynamics, fatigue, flow, and multiphysics adapters. | Each solver class has its own verified capability matrix and convergence evidence. |
| DE-703 | Stress/displacement/temperature/flow plots, probes, sections, animations, convergence, comparisons, and reports. | Reports separate solver failure, unconverged results, and valid computed results. |
| DE-704 | Parametric studies, sensitivity, topology optimization, generative candidates, and optimization constraints. | Candidates retain objectives/constraints and can become editable geometry with approximation reports. |
| DE-705 | Mechanism dynamics, building analytical exchange, and electronics thermal/mechanical coupling. | Coupling preserves units, boundary mappings, and revision lineage. |

Physical simulation and visual-effects simulation use different accuracy and validation criteria. They share geometry and scheduling but cannot substitute results for one another. A photorealistic deformation is not structural evidence.

### 5.8 Manufacturing and fabrication

| ID | Required capability | Acceptance workflow |
| --- | --- | --- |
| DE-800 | Machines, tools, holders, stock, fixtures, work offsets, setup orientations, and feeds/speeds inputs. | Every operation resolves its machine and setup dependencies. |
| DE-801 | Drilling, 2D/2.5D milling, 3-axis roughing/finishing, adaptive strategies, turning, mill-turn, and indexed/simultaneous multi-axis operations. | Paths are checked against benchmark parts and supported machine configurations. |
| DE-802 | Rest machining, tool/holder collision, stock removal, gouge detection, travel limits, simulation, setup sheets, and estimates. | An intentional collision/gouge produces a blocking diagnostic before release. |
| DE-803 | Versioned postprocessors, controller-specific output, post validation, and machine-aware verification. | Output is tied to a tested machine/post pair and a reviewed revision. |
| DE-804 | Sheet nesting, laser/waterjet/plasma profiles, kerf/lead-ins/tabs, and bend/fabrication outputs. | Nest dimensions and cutting contours match the source geometry and setup. |
| DE-805 | Additive orientation, supports, slicing adapters, layer preview, repair, material profiles, and 3MF/STL output. | Units, manifoldness, wall checks, and slicing settings are explicit. |
| DE-806 | Inspection plans, probing adapters, measured-versus-designed comparisons, and quality reports. | Measurement results retain coordinate alignment, units, uncertainty inputs, and source revisions. |

Generating a toolpath or code file is separate from sending commands to a machine. Machine execution requires an explicitly configured machine integration and its release authorization. Geometry changes invalidate affected operations and verification; stale output cannot be labeled current.

### 5.9 Electronics

| ID | Required capability | Acceptance workflow |
| --- | --- | --- |
| DE-850 | Schematic capture, symbols/libraries, connectivity, net labels, hierarchy, and electrical-rule checks. | A schematic change produces a traceable netlist update. |
| DE-851 | PCB footprints, placement, routing, layers, design-rule checks, clearances, and board geometry. | Supported rule violations are detected and tied to objects. |
| DE-852 | Mechanical enclosure/PCB coordination, component height and collision checks, and connector placement. | Board outline and component changes update the mechanical representation. |
| DE-853 | Gerber/drill, BOM, placement files, and versioned fabrication packages. | Outputs reopen in independent viewers and match the intended board revision. |

Electronics belongs to the complete scope because the Fusion replacement target includes connected electronic and mechanical design. Advanced analysis and vendor-specific libraries can use adapters but must not require a separate internal project model.

### 5.10 Rigging, animation, and effects

| ID | Required capability | Acceptance workflow |
| --- | --- | --- |
| DE-900 | Timeline, keys, curve/graph editor, interpolation, tangents, constraints, drivers, motion paths, markers, and clip sequencing. | Seeking any frame produces the expected deterministic evaluated state. |
| DE-901 | Skeletons, joint orientation, IK/FK, controllers, skinning, weight painting, blend shapes, deformers, and corrective shapes. | A rigged character retains deformation and control behavior after save/reopen. |
| DE-902 | Animation layers, nonlinear clips, retiming, retargeting, pose libraries, mocap import, and camera animation. | Multiple clips/layers can be edited without destroying source animation. |
| DE-903 | Cloth, hair/groom, particles, rigid/soft bodies, fluids/volumes, procedural effects, caching, and collision proxies. | Simulations record seeds, settings, dependencies, and reproducible cache manifests. |
| DE-904 | Procedural instancing, scattering, crowds, reusable scene assets, and level-of-detail policies. | Large instanced scenes do not duplicate full assets for every instance. |
| DE-905 | Product assembly animation, exploded sequences, architecture walkthroughs, and character/scene storytelling. | Engineering objects retain identity while animated presentation poses remain separate from design state. |

Rendering animation is part of the application's intended runtime. The specification itself does not create or render a video. Animation and physics caches are revision-bound jobs; scrubbing uses caches or deterministic evaluation, never accumulated wall-clock state.

### 5.11 Materials, lighting, and rendering

| ID | Required capability | Acceptance workflow |
| --- | --- | --- |
| DE-950 | PBR materials, node-based shading, textures, decals, displacement, material libraries, and overrides. | Physical properties and visual appearance remain separately editable. |
| DE-951 | Cameras, lenses, depth of field, exposure, HDR environments, sun/sky, analytic lights, and light linking. | Camera/lighting settings persist and work in headless render jobs. |
| DE-952 | Real-time shaded viewport, wireframe, technical styles, progressive path tracing, and CPU/GPU rendering adapters. | The same scene can be previewed and rendered with documented differences. |
| DE-953 | Render layers, passes/AOVs, object masks, motion blur, denoising, compositing outputs, and color management. | Passes and metadata are usable in external compositors. |
| DE-954 | Turntables, stills, animation/image sequences, panoramic outputs, render queues, and optional distributed rendering. | Jobs resume from completed outputs and record scene/revision/settings hashes. |
| DE-955 | Reusable visualization setups, review links, presentations, AR-ready export, and interactive scene review. | Simplified review assets retain correct scale and selectable object mapping. |

### 5.12 Shared projects and storage

| ID | Required product behavior | Acceptance workflow |
| --- | --- | --- |
| DE-970 | Share a project through an invitation. The recipient opens the browser interface, installs the engine/launcher if local processing is needed, and joins as a viewer or editor. | Two people connect to the same project without exchanging separate model copies. |
| DE-971 | Live peer-to-peer collaboration with shared cursors, selections, and simultaneous editing across workspaces. | Both people see accepted edits; conflicting changes are surfaced without losing work. |
| DE-972 | Follow the same camera, drawing view, section, or animation position, with independent navigation and presenter handoff. | Two collaborators inspect the same building or part while both retain editing access. |
| DE-973 | Optional native saving to Amazon S3, DigitalOcean Spaces, and supported S3-compatible storage. | Users choose their own storage inside the app and reopen the shared project from it. |
| DE-974 | Reference the same stored project and load only the models, assets, and history needed for the current work. Without cloud storage, retrieve needed content from the hosting peer. | Joining a large project does not require a full download; offline copies are optional. |
| DE-975 | Automatic Git-backed change history with plain-language History, Compare, Restore, and Versions controls. | Human and AI edits are tracked in the background without users manually making commits. |
| DE-976 | Optional Git repository as a project destination, with support for large design assets. | Connect a repository, save and synchronize in the background, and recover earlier versions through the app. |
| DE-977 | Keep modeling responsive during sharing, saving, and history updates; clearly show local save and shared sync status. | Slow or unavailable storage does not freeze the viewport or silently lose pending changes. |

Cloud storage and a Git repository are choices, not requirements for peer-to-peer work. Connected devices may cache needed data; complete project copies are optional. Git records history in the background while live collaboration carries current edits. Shared edits update dependent workspaces or mark their results as needing recalculation.

## 6 AI agent and automation contract

### 6.1 Agent abilities

The agent can inspect the project tree, query geometry, measure objects, read constraints, inspect dependencies, select entities, create/edit features, configure workspaces, run validators, launch bounded jobs, produce drawings, and compare revisions. Images, sketches, scans, text briefs, and existing models can inform a design, with inferred dimensions or assumptions labeled.

Tasks return structured editable artifacts. Generating an image of a bracket does not complete a request to create a bracket model. Generating a mesh does not complete a request for a constrained parametric solid unless the requested representation and editability are actually provided.

### 6.2 Command envelope

Every mutating tool uses a versioned envelope with an idempotency key, project ID, expected revision, operation, targets, typed parameters/units, and actor. Commands can be grouped into an atomic change set. Example:

```json
{
  "schema_version": 1,
  "command_id": "cmd-spanner-017",
  "project_id": "project-toolkit",
  "expected_revision": "rev-042",
  "operation": "feature.parameter.set",
  "targets": [{"entity_id": "feature-handle-pocket"}],
  "parameters": {
    "depth": {"value": 2.5, "unit": "mm"}
  },
  "constraints_to_preserve": ["jaw-opening", "mounting-datums"],
  "actor": {"kind": "agent", "session_id": "session-009"}
}
```

The execution pipeline is inspect → plan → prepare → preview → validate → commit → recompute → report. Preview returns geometry differences, parameter changes, downstream effects, validation results, and estimated job costs. Execution records the actual commands so replay does not depend on reproducing model reasoning.

### 6.3 Authority and failures

Manual commands and authorized agent commands can commit reversible local edits without repetitive confirmations. Users may choose review-every-change, bounded autonomy, or named task authorization. Release/export actions honor configured authority. Publishing projects, committing to shared protected revisions, spending remote-compute budget, or executing a machine job has a specific visible scope.

Return errors such as `STALE_REVISION`, `AMBIGUOUS_SELECTION`, `UNRESOLVED_REFERENCE`, `UNSATISFIABLE_CONSTRAINT`, `INVALID_GEOMETRY`, `UNSUPPORTED_REPRESENTATION`, `SOLVER_DID_NOT_CONVERGE`, and `STALE_DERIVED_OUTPUT`. A failed change set leaves the last valid project state intact. Repair proposals are new reviewable actions rather than hidden destructive fallbacks.

Embedded text, imported files, external metadata, and retrieved documents are untrusted data. Agent tools enforce resource and permission scopes. Provider keys are stored in platform credential storage. Remote AI receives only the selected project scope, and local-model operation is supported through the same interface.

### 6.4 Inspectable reasoning outputs

The UI shows design assumptions, selected targets, changes, checks performed, unresolved questions, and evidence. It need not expose internal model reasoning. Statements about weight, fit, strength, energy, cost, and manufacturability identify the measurements, material data, study, or rule set that supports them.

## 7 Engine implementation architecture

```text
Browser / optional native client / CLI / agent
                 |
         Typed command service
                 |
 Project graph + revisions + units + dependencies
                 |
 Geometry / constraints / BIM / mesh / animation
                 |
 Simulation / CAM / electronics / rendering
                 |
 Local workers / optional remote compute / file adapters
```

### 7.1 Rust core and execution

Use a Rust workspace with clear ownership for graph state, transactions, entity IDs, schemas, storage, scheduling, jobs, and plugin boundaries. Keep model mutations on an ordered transaction boundary; evaluate independent immutable jobs concurrently. Do not assume a native kernel is thread-safe; serialize unsafe contexts or isolate workers per document/job as required by testing.

Exact geometry kernels may run through a C-compatible FFI layer with explicit ownership, cancellation boundaries, and structured failures. Open CASCADE Technology is an initial candidate for evaluating exact modeling and interchange, alongside licensed alternatives if corpus results require them. This is an evaluation choice, not a promise of full workflow coverage. [Open CASCADE Technology](https://occt3d.com/open-cascade-technology/index.html).

Product-owned layers include stable references, design intent, project semantics, dependency invalidation, cross-workspace behavior, transaction history, agent operations, and diagnostics. Reusing a tested geometric algorithm does not transfer project ownership to a separate application.

### 7.2 Viewport and clients

Use React and TypeScript in a normal browser for the main interface, including workspace panels, chat, properties, and history. Use browser GPU rendering for the interactive viewport on supported devices. The Rust engine remains a separate process with a typed API, serving local interface assets for offline use and supporting explicitly authorized remote/peer connections.

Expensive modeling, simulation, manufacturing, and rendering jobs can run in Rust while the browser remains the interface. Introduce the optional native viewport/client when interactive performance, graphics capabilities, or device integration requires it. Benchmark representative workloads to decide when this path is useful.

“Open Native View” launches an installed companion window connected to the same engine, revision, selection, camera, and collaboration session. It is a separate native surface; a normal browser does not silently become a native window. Return to the browser without exporting or recreating the project. Native controls cover the interactions needed for the supported heavy workflows; property and agent panels may remain in the browser.

Prototype browser/native rendering code reuse and efficient binary scene transfer. Keep the scene evaluation and project rules independent of both clients. Closing either client preserves engine-owned state and authorized jobs. Authenticated sessions, origin checks, scoped access, and supported browser connection permissions govern local engine access.

Use instancing, hierarchical culling, progressive loading, bounded tessellation caches, selection acceleration structures, and adjustable levels of detail. GPU precision limitations cannot alter authoritative engineering geometry. Browser/tablet clients use the same schema and command semantics even when heavy work is assigned to a worker.

### 7.3 Storage format

Use an open, documented native project package with a working directory/store for active editing. Proposed components:

```text
manifest.json             Schema, project identity and compatibility
project.sqlite            Entity metadata, references and revisions
objects/                  Content-addressed exact/mesh/field payloads
assets/                   Textures, libraries and source references
history/                  Command records and checkpoints
derived/                  Disposable tessellation and job caches
reports/                  Validation and release manifests
```

The portable package is an atomic snapshot, not a database edited directly inside a compressed archive. Flush/checkpoint the working store before packaging. Source files can be embedded or referenced with explicit portability status. Large caches can be excluded from a portable archive and regenerated.

Schema migrations preserve a recovery copy, validate referential integrity, and record the migration chain. Missing workspaces/plugins may leave typed entities unavailable for editing, but must not cause silent deletion. Unknown extension data is preserved when safe.

### 7.4 Collaboration

Peer-to-peer collaboration is part of the foundation. Shared sessions provide identities, roles, comments, presence, synchronized views, and simultaneous editing. Cloud storage and Git remotes are optional project destinations; a vendor-hosted revision service is not required for ordinary peer-to-peer work.

Use optimistic revision checks and dependency-aware merge for model transactions. Concurrent edits to disjoint properties may merge; competing edits to the same topology or constraint group require explicit resolution. Text-oriented CRDTs may suit comments and notes, but do not establish geometric validity. Offline branches reconnect through the same validation and merge rules.

Shared review may stream derived geometry and properties without exposing the entire native project. Access controls distinguish viewing, commenting, editing, publishing, exporting, and administrative operations. Downloaded exports remain outside later link revocation.

### 7.5 Extension and compute contracts

Plugins declare commands, entity types, UI panels, file handlers, jobs, permissions, schema migrations, supported platform versions, and licenses. Isolate unsafe/native extensions in workers; use capability-limited runtimes where feasible. Prevent plugins from bypassing command validation or corrupting shared state.

Compute jobs declare input revision hashes, solver versions, seeds, hardware needs, memory/time limits, cancellation behavior, expected artifacts, and validation status. Remote jobs show cost/resource estimates, use authorized scopes, and never publish results into a different revision silently.

## 8 Interoperability and migration

Format support has separate grades: view, geometry import, semantic import, editable feature import, export, and round-trip. Each adapter has a tested version range, units policy, supported object list, and loss report. Opening a model is not evidence that its original design history survived.

| Format family | Target behavior | Main verification |
| --- | --- | --- |
| STEP and IGES | Exact-geometry exchange; supported assemblies and metadata/PMI according to the chosen profile. | Units, topology, tolerance, assembly hierarchy, and profile-specific data. |
| DXF and DWG | Drafting exchange through tested parsers or licensed SDKs. | Entities, layers, blocks, fonts, dimensions, layouts, references, and version support. |
| IFC and BCF | Building geometry, identity, relationships, selected semantics, and issues. | GUIDs, placement, spatial hierarchy, property mappings, and declared exchange requirements. |
| STL, OBJ, PLY, 3MF | Mesh/print/scan exchange with explicit unit and material limits. | Topology, normals, scale, colors, material mappings, and manifoldness. |
| glTF/GLB, USD, Alembic | Scene, animation, cache, and visualization exchange by supported schema/profile. | Instancing, transforms, materials, animation channels, and time sampling. |
| FBX | Tested scene/rig/animation interchange via an appropriate adapter. | Skeletons, weights, transforms, animation, and documented losses. |
| 3DM | Geometry and supported metadata exchange; procedural/plugin data treated separately. | NURBS/SubD/mesh fidelity and unsupported object preservation/reporting. |
| RVT/RFA, IPT/IAM, SLDPRT/SLDASM, Fusion archives, SHAPR | Prioritized migration investigations using supported SDKs or user-authorized neutral exports. | Actual editability, configurations, relationships, and history fidelity by source version. |
| MA/MB and MAX | Scene migration investigations with vendor-assisted export or supported SDK routes. | Geometry/material/rig/animation survival; custom plugins and scripts reported explicitly. |
| Gerber/drill and board exchange formats | Electronics fabrication and coordination adapters. | Layer mapping, units, apertures, placement, and revision alignment. |
| PDF/SVG and images | Documents, underlays, drawings, and presentation outputs. | Correct scale, fonts, vectors, color, and embedded source metadata policy. |

Native proprietary formats can require commercial technology or the original application for export. The fallback is an explicit import route with a fidelity report, not an invented claim of lossless native support. Plugins written for other applications need compatible APIs or ports; file import alone cannot run them.

## 9 Performance budgets and scaling

These are proposed targets. Benchmark on a documented 16 GB Apple Silicon laptop and a 16 GB Windows x64 workstation/laptop with a named GPU and local SSD. Larger production scenes use a separately defined 64 GB workstation profile. Publish cold/warm conditions, p50/p95, GPU memory, worker memory, and fidelity settings.

| Workload | Proposed initial gate |
| --- | --- |
| Browser interface ready | At most 3 seconds with browser and engine running and local interface assets available; measure engine and native-client startup separately. |
| Ordinary object selection and property feedback | At most 100 ms p95 in reference scenes. |
| Camera orbit of 1 million visible triangles with defined shading | Target 60 Hz on the reference GPU; maintain responsive input with adaptive detail. |
| Incremental edit in a 100-feature mechanical reference part | At most 500 ms p95 for designated simple parameter changes; report expensive operations separately. |
| Open 1,000-instance assembly using 100 unique parts | First useful viewport within 3 seconds; progressive loading remains cancellable. |
| Idle application memory | At most 400 MB attributable to the app tab, engine, and required workers; browser baseline recorded separately and optional solvers unloaded. |
| Long jobs | Immediate job acknowledgment, visible progress where measurable, cancellable at documented safe points. |
| Recovery | Every acknowledged transaction recoverable; no partially committed geometry graph. |

Use benchmarks for large drawings, 10,000+ component assemblies, building models, high-resolution sculpting, dense point clouds, and rigged/instanced scenes before claiming professional-scale support. Track topological robustness and output quality alongside speed; a faster invalid result does not pass.

## 10 Validation and acceptance

### 10.1 Engineering invariants

- Exact geometry must satisfy declared manifoldness, orientation, closure, intersection, and tolerance requirements for the operation.
- Constraints must expose remaining degrees of freedom, inconsistencies, and solver failure.
- Rebuilds must preserve intended references or surface a repairable break.
- All quantities and transforms must preserve units and coordinate frames across adapters.
- Derived artifacts must identify their source revision, configuration, and generating settings.
- Undo/redo, branching, save/reopen, and migration must preserve semantic design state.
- Invalid, stale, approximate, and unsupported results must be distinguishable from verified results.

### 10.2 Complete workflows

| Scenario | Required result |
| --- | --- |
| Adjustable spanner | Agent/manual sketch and solid creation; editable dimensions; assembly motion; mass properties; drawings; machining setup; render. Changing the jaw dimension propagates or marks affected outputs stale. |
| Sheet-metal enclosure with PCB | Mechanical geometry, board outline, connector alignment, clearance checks, flat pattern, BOM, drawings, and fabrication package remain linked. |
| Multi-storey building | Architecture, structure, MEP, room/area schedules, quantities, clashes, IFC exchange, and walkthrough share one project. A level change updates dependent objects. |
| Freeform consumer product | NURBS/SubD design, continuity analysis, shell/thickness, exact manufacturing representation where valid, textured render, and conversion-deviation evidence. |
| Animated character | Mesh authoring, sculpt detail, retopology, UVs, rigging, skinning, animation clips, effects caches, and rendered frames survive reopening and supported scene exchange. |
| Large product scene | Reused assembly definitions, lightweight loading, materials, camera animation, and render jobs operate with bounded memory. |
| Concurrent team revision | Two users make disjoint edits that merge; a conflicting shared-face edit is surfaced without silent damage. |
| Interrupted session | Termination during save/rebuild/import recovers the last acknowledged valid state and reports unfinished jobs. |

### 10.3 Test strategy

Maintain analytic primitives, adversarial geometry, licensed real-world models, drawing references, BIM exchange fixtures, solver benchmarks, machine/post fixtures, and animation/render scenes. Differentially compare geometry and exchange outputs with independent tools where available. Use numeric properties and topology in addition to screenshots.

Include property-based geometry tests, schema/command fuzzing, mutation/rebuild stress, import/export loops, crash injection, revision conflict tests, performance regression workloads, input/accessibility tests, and plugin isolation checks. Numerical solvers and manufacturing postprocessors require domain-specific verification in addition to application tests.

Agent evaluations score actual successful tasks, preserved constraints, model validity, unintended changes, repair quality, and supported claims. “The model looks plausible” is not a passing engineering criterion.

## 11 Release scope and decisions

The [build plan](BUILD_PLAN.md) sequences the full application. Mechanical workflows provide the first vertical slice because they exercise units, constraints, exact geometry, shared views, commands, and derived outputs. BIM and animation are included in the schema and fixtures early so the foundation cannot become a mechanical-only file model.

Decisions for implementation spikes:

- Exact-kernel choice, topology mapping strategy, and native-library concurrency behavior.
- Constraint solver coverage, diagnostics, expression engine, and numerical tolerances.
- GPU viewport/client integration and portable selection/rendering architecture.
- Mesh/SubD and exact-geometry correspondence policies.
- IFC and native-format adapter coverage, licensing, and migration costs.
- Simulation, CAM, electronics, character/effects, and rendering engines by verified capability.
- Local storage checkpoints, portable package format, and semantic merge strategy.
- Platform rollout, staffing, paid services, pricing, and commercial packaging.

The product is complete against this specification only when all required workspaces can complete their acceptance workflows through the common project model. Initial releases are useful milestones toward that result, not claims that a simple modeler already replaces all nine products.
