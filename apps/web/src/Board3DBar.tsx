// The board in the 3D view: show and hide its mask, silkscreen, copper and
// parts (by type: SMD, through-hole, other, do-not-place), each part's
// bounding box, and save the view as a PNG or the board as drawn as glTF.
// View only: nothing here is saved in the project.

import { useEffect, useState } from "react";
import { Icon } from "./icons";
import { type BoardShow, pcbLooks } from "./pcb3d";
import "./PcbTools.css";

function download(blob: Blob, name: string) {
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 2000);
}

export function Board3DBar(props: { snapshot: () => string | null; hidden: boolean }) {
  const [drawn, setDrawn] = useState(() => pcbLooks.drawn());
  const [show, setShow] = useState<BoardShow>(() => pcbLooks.getShow());
  const [open, setOpen] = useState(false);
  useEffect(() => pcbLooks.subscribe(() => setDrawn(pcbLooks.drawn())), []);
  if (props.hidden || drawn.length === 0) return null;
  const toggle = (k: keyof BoardShow) => {
    const now = pcbLooks.getShow();
    const s = { ...now, [k]: !now[k] };
    setShow(s);
    pcbLooks.setShow(s);
  };
  const chip = (k: keyof BoardShow, label: string, title?: string) => (
    <button key={k} type="button" className={`chip ${show[k] ? "on" : ""}`} aria-pressed={show[k]} onClick={() => toggle(k)} title={title}>
      {label}
    </button>
  );
  const gltf = async () => {
    const { GLTFExporter } = await import("three/examples/jsm/exporters/GLTFExporter.js");
    const THREE = await import("three");
    const root = new THREE.Group();
    for (const d of drawn) root.add(d.look.clone());
    // Millimetres, z up → glTF metres, y up.
    root.rotation.x = -Math.PI / 2;
    root.scale.setScalar(0.001);
    const out = await new GLTFExporter().parseAsync(root, { binary: true });
    download(new Blob([out as ArrayBuffer], { type: "model/gltf-binary" }), "board.glb");
  };
  const png = () => {
    const url = props.snapshot();
    if (!url) return;
    const a = document.createElement("a");
    a.href = url;
    a.download = "board.png";
    a.click();
  };
  return (
    <div className="b3d" role="toolbar" aria-label="Board in 3D">
      <button type="button" className="chip on" onClick={() => setOpen(!open)} title="What the 3D view shows of the board">
        <Icon name="chip" /> Board{open ? "" : " …"}
      </button>
      {open && (
        <>
          {chip("mask", "Mask")}
          {chip("silk", "Silkscreen")}
          {chip("copper", "Copper")}
          {chip("bodies", "Parts")}
          <span className="sep" />
          {chip("smd", "SMD")}
          {chip("tht", "Through-hole")}
          {chip("other", "Other")}
          {chip("dnp", "DNP", "Parts marked do-not-place")}
          {chip("boxes", "Boxes", "Each part's bounding box")}
          <span className="sep" />
          <button type="button" className="chip" onClick={png} title="Save the view as a picture">
            PNG
          </button>
          <button type="button" className="chip" onClick={() => void gltf()} title="Save the board as drawn (glTF binary)">
            glTF
          </button>
        </>
      )}
    </div>
  );
}
