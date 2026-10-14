#!/usr/bin/env node
// Puts the robot arm in a named pose: sets each joint's angle (one change per
// joint, each undoable). The joints are the assembly's hinges, scrubbable in
// the view; these are a few useful positions to start from.
//
//   node examples/robot-arm/pose.mjs <home|reach|pick|stow|wave> [--project name] [--url ws://127.0.0.1:7420/ws]

export const POSES = {
  // degrees: base yaw, shoulder, elbow, wrist, gripper (0 closed … 30 open)
  home: { "Base yaw": 0, Shoulder: 0, Elbow: 0, Wrist: 0, Grip: 0 },
  // Full horizontal reach (the payload check's pose).
  reach: { "Base yaw": 0, Shoulder: 90, Elbow: 90, Wrist: 0, Grip: 10 },
  // Reaching down to the bench in front, gripper open and pointing down.
  pick: { "Base yaw": -35, Shoulder: 40, Elbow: 20, Wrist: -70, Grip: 25 },
  // Folded back over the base.
  stow: { "Base yaw": 0, Shoulder: -30, Elbow: -55, Wrist: 60, Grip: 0 },
  // Raised, turned to the side.
  wave: { "Base yaw": 60, Shoulder: -15, Elbow: 60, Wrist: 35, Grip: 30 },
};

const args = process.argv.slice(2);
const flag = (name, dflt) => {
  const i = args.indexOf(name);
  return i >= 0 ? args.splice(i, 2)[1] : dflt;
};
const base = flag("--url", "ws://127.0.0.1:7420/ws?user=ise-ai");
const project = flag("--project", null);
const pose = POSES[args[0]];
if (!pose) {
  console.error(`usage: pose.mjs <${Object.keys(POSES).join("|")}> [--project name] [--url ws://…]`);
  process.exit(1);
}
const url = project ? `${base}${base.includes("?") ? "&" : "?"}project=${encodeURIComponent(project)}` : base;
const ws = new WebSocket(url);
let revision = 0;
let request = 1;
const waiting = new Map();
const snapshot = new Promise((ready) => {
  ws.onmessage = (e) => {
    const m = JSON.parse(e.data);
    if (m.type === "snapshot") {
      revision = m.revision;
      ready(m);
    } else if (m.type === "changed") revision = Math.max(revision, m.revision);
    else if (m.type === "result") {
      if (m.ok && typeof m.value?.revision === "number") revision = Math.max(revision, m.value.revision);
      waiting.get(m.request_id)?.(m);
    }
  };
});
await new Promise((r, j) => ((ws.onopen = r), (ws.onerror = () => j(new Error(`cannot reach ${url}`)))));
const s = await snapshot;
const joints = new Map(s.entities.filter((e) => e.components["assembly.joint"]).map((e) => [e.components["shape.parameter_set"]?.name, e.id]));
for (const [name, angle] of Object.entries(pose)) {
  const id = joints.get(name);
  if (!id) {
    console.error(`no joint “${name}”`);
    continue;
  }
  const r = await new Promise((done) => {
    const i = request++;
    waiting.set(i, done);
    ws.send(JSON.stringify({ type: "commit", request_id: i, operation: "parameters.set", parameters: { target: id, values: { angle } }, expected_revision: revision }));
  });
  console.log(r.ok ? `✓ ${name} ${angle}°` : `✗ ${name}: ${r.error?.message}`);
}
ws.close();
