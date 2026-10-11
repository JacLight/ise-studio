# Design Engineering

One AI-first application, one project, and one shared engine for drafting, mechanical design, architecture and BIM, manufacturing, freeform modeling, animation, and rendering.

The complete target covers workflows associated with AutoCAD, Revit, Inventor, Fusion, Maya, 3ds Max, SOLIDWORKS, Rhino, and Shapr3D. These become workspaces inside the same product, with shared objects, history, commands, and derived outputs.

Status: specification complete for initial implementation planning; application development has not started.

Interface: normal browser with React and TypeScript, connected to an independent Rust engine. An optional native viewport/client uses the same project for demanding 3D workflows.

- [Full product and technical specification](SPEC.md)
- [Build sequence and release gates](BUILD_PLAN.md)

This is separate from the [PDF Editor project](../pdf-editor/README.md). The folder name is a working name. “Shpra” in the original request is interpreted as Shapr3D.

- [Interactive UI mockup](MOCKUP.html) — canvas-first concept with hideable controls and mini, docked, floating, expanded and hidden AI chat.

- [Design preferences, choices, and rationale](DESIGN.md) — self-contained design guide for this application.

Full local UI path: `/Users/imzee/projects/design-engineering/mockup/index.html`. Open it directly in a browser.
