# Design Engineering Interface Design

Version: 2.0. Date: October 8, 2026. Working identity: Forma.

This is the self-contained design guide for this project. It records interface preferences, choices, rationale, state behavior and implementation boundaries. The complete functional target is in [SPEC.md](SPEC.md); delivery sequencing is in [BUILD_PLAN.md](BUILD_PLAN.md).

Local interactive UI: `/Users/imzee/projects/design-engineering/mockup/index.html`.

[Open the UI](mockup/index.html) or use [MOCKUP.html](MOCKUP.html). Forma is a working name, not a final trademark decision.

## Core design decision

The work fills the application. AI conversation is a first-class way to operate on that work. Every piece of interface can be put away: the tool rail, project identity, sharing controls, view bar, inspectors, and the assistant itself. The default is a quiet canvas with a small AI composer. Opening the application must not produce a wall of panels, workspace tabs, tool categories, or feature cards.

The user's October 8 reference screenshots establish the interaction direction: a light dotted canvas, slim floating tool strip, floating contextual cards, and an AI assistant that can shrink, dock, float, or disappear. The reference is an interaction pattern, not content to copy into the product. The previous graphite interface with permanent navigation and assistant columns is superseded by this design.

## Preferences, choices, and reasons

| Preference | Choice | Reason |
| --- | --- | --- |
| AI first | A directly available prompt with a complete conversation behind it | Intent is an entry point into real editing, not a help widget |
| Work comes first | Canvas occupies the window; controls float above it | Protects usable working space |
| Everything can hide | Separate close controls and a global focus command | Users choose how much interface is visible |
| Powerful without visual clutter | Commands exposed through chat, contextual actions, and searchable manual tools | Capability does not require permanent screen occupancy |
| Predictable conversation | One assistant session per project; modes only change presentation | Moving the assistant should never start over |
| Direct manipulation | Selecting and editing the work remains available | Conversation and manual work complement each other |
| User control | Proposed changes, clear scope, undo, recoverable history | Users can inspect and refine what the agent does |
| Shared work | Presence, invite and follow-view are available on demand | Collaboration should not dominate a solo session |
| Storage independence | Local, peer and optional cloud/repository routes use the same workspace | Location should not dictate the editing experience |
| Independent engine | Browser UI operates through a typed Rust-engine API | UI presentation cannot become the source of engineering or document truth |

## Visual language

Use a light, neutral canvas with a very subtle dot grid where useful for spatial orientation. PDF page interiors retain their original appearance. 3D space uses neutral lighting and a faint ground grid. Both grids can be hidden in the production interface. White floating surfaces have fine borders and soft shadows. Purple is reserved for AI entry points, current actions and selections; it should not wash the entire screen in a brand color.

| Token | Starting value | Purpose |
| --- | --- | --- |
| Workspace | `#FAFBFC` | Quiet overall surface |
| Floating surface | `#FFFFFF` | Assistant, tool cards and small controls |
| Border | `#E8E9EE` | Subtle separation |
| Main text | `#343743` | Titles and important control labels |
| Supporting text | `#737985` | Descriptions and secondary controls |
| AI accent | `#7352D6` | Active controls and primary actions |
| Selected surface | `#F3EFFA` | Low-intensity selection background |
| Canvas dots | `#CFD3DC` | Spatial orientation outside the work |

These are design starting points. Meaningful text needs contrast validation before release; the prototype contains some deliberately subdued decorative labels that still need that pass. Application appearance never silently changes a document's exported colors or a model's assigned materials. A dark theme can come later with the same hierarchy; dark panels are not the defining identity.

Use DM Sans or the native system sans-serif for interface text. The mockup's document uses Manrope as fictional report styling, not as a mandated font for imported content. Package licensed fonts for production offline use. The prototype uses an optional Google Fonts stylesheet and falls back to installed fonts without it. Standard text should be readable at 12–14 px on desktop, with 16 px input text on touch devices where needed. Let user text scaling increase control sizes; do not reduce labels to keep a preferred layout.

Use restrained 1.5–1.75 px line icons, consistent silhouettes, descriptive tooltips and accessible names. A minimal interface must not require memorizing unlabeled symbols. Tool buttons need larger hit areas than the visible icon, especially on touch. Rounded corners distinguish movable cards from content; avoid decorative gradients except a very restrained AI accent.

## Canvas and chrome

The default desktop consists of a small project capsule at the upper left, optional sharing controls at the upper right, a narrow floating tool rail at the left, a compact view strip near the bottom, and the AI mini composer near the lower right. None is a full-width ribbon. Tool cards are closed initially. There is no mandatory left navigator or permanent right properties column.

The project capsule exposes the project name and a small menu. The two product names in the prototype's switcher are for reviewing the concepts together; the production apps remain separate products. The view strip contains only frequent navigation and recovery actions. Storage setup, history, export and specialized capabilities live behind their entry points or in command search. Tool discovery in the full product must support keyboard search, natural-language intent and contextual controls, without turning the canvas into a gallery of feature tiles.

Each control group has its own hide action. Focus mode hides all chrome and cards together, including AI, without deleting their state or stopping a running job. A small Restore control remains as an exit, and Escape restores access. F toggles focus when the user is not typing. A toggles AI visibility. Normal Tab navigation remains available; it is not stolen for focus mode. Commands must not interfere with content editing or browser shortcuts.

Closing an inspector only closes its presentation. Selection, edits and pending operations continue to exist. Multiple inspectors can be supported in production, but none opens by default and users must always be able to get back to an uncluttered canvas. The current prototype uses one movable tool card at a time to make this behavior concrete.

## First-class AI conversation

| State | Presentation | Transition and retained state |
| --- | --- | --- |
| Mini | Compact prompt, small assistant header, send and voice entry | Opens conversation when submitted; retains the draft |
| Docked | Full-height side conversation | Reserves canvas width; retains camera, selection, history and pending proposal |
| Floating | Movable, resizable conversation card | Can be placed over available space; retains the same conversation |
| Expanded | Large conversation surface | Useful for longer review; restores to floating mode |
| Hidden | Small AI launcher | Reopens the session; does not erase messages or cancel work |
| Voice | Voice presentation inside the assistant | Returns to the same text conversation |

Mini is the default. Full chat includes the current document or model context, user messages, assistant replies, proposal review and a persistent composer. Its header has explicit minimize, dock/float, expand and hide controls. A conversation is a working surface, not a temporary drawer. Repeatedly changing the presentation must not remount the renderer, reset the camera, lose a draft, clear the conversation, reset page scroll, or discard a pending change.

Floating panels move by their headers. The prototype also allows keyboard arrow movement from a focused header. The production app should remember position and size by workspace, clamp restored panels into the current screen, and provide easy reset after display changes. The prototype persists the floating assistant position, messages, draft and main presentation mode; manually resized dimensions and tool-card positions are not persisted yet.

The empty conversation uses one short invitation and a few relevant text suggestions. Once work starts, the conversation takes priority. Do not populate it with permanent menus, analytics, feature galleries or introductory marketing.

The agent should read the selected content and project context, propose structured commands, explain consequential effects concisely and operate through the same engine API as manual actions. AI execution is never a second source of project state. Show a change where it affects the work and let users accept, adjust or undo it. Approval requirements depend on the operation, project policy and prior authorization; routine reversible edits do not need repeated confirmations in production. The prototype explicitly uses Apply/Discard so the transition is reviewable.

Streaming, cancellation, retries, background work and partial failure must be real states. Hiding chat must not imply cancellation. Agent tool errors should explain which requested result remains incomplete. Do not present fabricated successful checks, saves or collaborations.

Voice is another way to use the same assistant. Production voice needs explicit microphone permission, visible listening/recording state, mute and end controls, a readable transcript, interruption handling and text parity. The mockup is a static voice-mode preview with the microphone off; it does not access audio hardware or implement speech recognition.

## Collaboration, storage and history

The sharing control should open a concise invitation flow with access roles and an optional follow-view action. Each collaborator can keep their own panel arrangement. Following a peer synchronizes the relevant page/camera and selection, not another person's entire interface layout. It must be possible to leave follow mode without leaving the project. Shared edits need actor identity, permission enforcement, conflict handling and reconnect recovery.

Native peer-to-peer collaboration is a core product requirement. Optional S3-compatible storage, DigitalOcean Spaces and Git repositories provide shared storage/history choices; they are not prerequisites for editing. Referencing the same object-store location does not itself synchronize edits: the engine's collaboration protocol still coordinates operations and revisions. Peers may fetch the working set they need rather than cloning all project assets. Offline availability and cache size should be explicit preferences.

Keep credentials and synchronization details out of normal canvas work. Setup lives in project settings. Background status must distinguish saved locally, delivered to peers, uploaded to object storage and pushed to a repository. Do not reduce distinct guarantees to a misleading universal Saved label. Detailed diagnostics should be available without demanding attention continuously.

History should read as useful changes: an edited paragraph, changed dimension, restored version or accepted AI operation. Users should not need Git terminology to recover their work. Git-backed checkpoints and branches belong behind meaningful product actions; live collaboration and undo must not rely on making every keystroke a Git commit.

## Responsive behavior and accessibility

Desktop docking reserves width for chat; floating and mini modes overlay the canvas. On a narrow screen, full chat becomes a bounded overlay and closes back to the work, rather than squeezing the canvas beside a wide sidebar. Tool cards remain reachable and scroll independently. No mode should strand controls outside the window after resizing.

All mode changes need keyboard equivalents and sensible focus handling. Closing a card should return focus to its opener in the production implementation. Dialogs must trap focus while open, expose labels, and return focus on close. The prototype uses native dialog elements. Do not convey selection, collaborator identity, validation failure or job state by color alone. Announce results without rereading an entire long conversation on every token.

Respect reduced motion, use brief transitions for layout changes, and avoid perpetual animation for decoration. Optional turntable motion is disabled by reduced-motion preference. A voice visual must support a static representation. Test browser zoom, screen-reader navigation, high contrast, touch targets, long translated strings and keyboard-only operation before accepting production layouts.

## Performance and engine separation

The browser manages layout, interaction, chat presentation and viewport composition. The Rust engine owns the authoritative project model, commands, transactions, validation, history, storage and collaboration. The same API supports a normal browser and optional native clients. Tauri may package a client later, but it is not required to use either application in a browser.

Keep rendering and expensive operations off the UI's critical path. Virtualize long conversations and navigators, load only the needed assets, cache derived previews and keep background jobs cancellable. Panel movement should only update panel layout. Changing assistant presentation must not recreate the document or 3D renderer. Selection and view are explicit session state. Define measured performance budgets against representative small and large projects before making speed claims.

## Prototype boundaries and review

Open the local HTML directly. The concept has real browser interaction, local state persistence, and the controls described below. It does not connect to the Rust engine, a language model, microphone, a peer, Git, an object store or real project files. Storage and sharing dialogs show the intended flow and explicitly identify their preview status. Local browser storage is a demo convenience, not the product save format.

The interaction check covers assistant transitions, draft retention, separate conversations for each app, safe message rendering, unchanged canvas identity across mode changes, direct editing, proposals, undo, properties validation, hiding/restoring controls and keyboard panel movement. The 3D scene is constructed with real Three.js objects and a renderer stub in the automated check. Browser layout, actual GPU output, touch behavior and screen-reader usability still need review in a real browser; this environment could not launch one.

## Model-specific experience

The shared 2D/3D model is the working surface. A neutral scene, restrained ground grid and light selection outlines help geometry read clearly. Avoid decorative dashboards around the model. Dimensions, snaps, constraints and analysis overlays appear when relevant and can be hidden independently. Use engine-derived units and values in production; UI labels must never become model truth.

The prototype opens an architectural pavilion because building design is explicitly part of the product. Its objects include a roof canopy, floors, glazing, facade fins and foundation. Production architecture/BIM includes sites, stories, walls, floors, roofs, openings, building systems, spaces, schedules and drawing views as defined by SPEC.md. They share the same project with mechanical parts, assemblies, freeform surfaces, materials, animation and manufacturing operations.

Switching workspaces changes the available tools and view, not the project identity. Modeling, architecture, drafting, simulation, manufacturing, animation and rendering should be discoverable through intent, context and a compact workspace switcher. Specialized node, curve, timeline or schedule editors are summonable work surfaces. An animator can keep a timeline open while working; another user need never see one. No discipline should impose its permanent panel arrangement on everyone else.

Selecting an object reveals contextual actions or its requested inspector. Numeric dimensions, unit handling, snapping, constraints and precise properties must remain available alongside chat. The production engine checks validity and dependencies after each applicable edit, distinguishes pending recomputation from completed results, and exposes failed features clearly. A plausible image is not evidence of a valid solid, a safe structure or a usable toolpath.

The local Three.js scene supports orbit by dragging, zoom by wheel or view controls, object picking, perspective/front/top views, wireframe, and a turntable. A movable properties card edits canopy width, pavilion height and structure material. AI demonstrations prepare the same changes with review and undo. Width is constrained to 8–20 m and height to 4–10 m for this sample. These bounds are demonstration choices, not engineering requirements. Height scales the illustrative model; no structural or BIM constraint solver is connected.

Viewport export produces a PNG image, not a CAD file. There is no geometry kernel, simulation, fabrication or BIM database behind the prototype. Production constraints, topology, native formats, interoperability and the complete application scope remain in SPEC.md.

## Browser and native view

Use the normal browser for the main interface and supported GPU workloads. Measure representative model sizes, materials and interactions before choosing where native rendering is needed. An optional native viewport connects to the same Rust engine, revision, selection, camera and collaboration session. Opening it must not require exporting a second project or abandoning the browser conversation. Native presentation is a capability choice, not a separate application model.

## Product acceptance examples

1. Open the pavilion: the model and mini AI composer appear without a project tree or properties column.
2. Ask to widen the canopy: review the proposed value, apply it, see the model change and undo it.
3. Move chat through docked, floating, expanded and hidden states: keep the same renderer, camera, conversation and selected object.
4. Open the objects card or properties card, make a precise edit and close the card again.
5. Enter focus mode: show only the model and a discreet restore control.
6. In production, move between architectural, mechanical and visualization work: preserve project relationships and expose only the tools needed for the current work.
