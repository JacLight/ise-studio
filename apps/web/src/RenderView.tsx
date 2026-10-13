// Render mode: the view path-traced live over the model (orbit and zoom as
// usual; each move starts the image over), and Save PNG at a chosen size.

import { useEffect, useRef, useState } from "react";
import { type Backdrop, LiveRender, renderImage, type RenderSource } from "./render";

const SIZES: { label: string; width: number; height: number }[] = [
  { label: "1920 × 1080", width: 1920, height: 1080 },
  { label: "2000 × 2000", width: 2000, height: 2000 },
  { label: "2560 × 1440", width: 2560, height: 1440 },
  { label: "3840 × 2160 (4K)", width: 3840, height: 2160 },
];

export function RenderView(props: { source: () => RenderSource | null; name: string; backdrop: Backdrop; onBackdrop: (b: Backdrop) => void; onClose: () => void }) {
  const host = useRef<HTMLDivElement>(null);
  const [samples, setSamples] = useState(-1);
  const [size, setSize] = useState(0);
  const [quality, setQuality] = useState(256);
  const [saving, setSaving] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const source = props.source();
    if (!source || !host.current) return;
    let live: LiveRender | null = null;
    try {
      live = new LiveRender(source, setSamples, props.backdrop);
      live.canvas.className = "render-canvas";
      host.current.prepend(live.canvas);
    } catch (e) {
      setError(String(e));
    }
    return () => {
      live?.canvas.remove();
      live?.dispose();
    };
  }, [props.source, props.backdrop]);

  const save = async () => {
    const source = props.source();
    if (!source || saving !== null) return;
    setSaving(0);
    setError(null);
    try {
      const { width, height } = SIZES[size];
      const blob = await renderImage(source, { width, height, samples: quality, backdrop: props.backdrop }, setSaving);
      const a = document.createElement("a");
      a.href = URL.createObjectURL(blob);
      a.download = `${props.name || "Forma"} render ${width}x${height}.png`;
      a.click();
      setTimeout(() => URL.revokeObjectURL(a.href), 10_000);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(null);
    }
  };

  return (
    <div className="render-view" ref={host}>
      <div className="render-bar" role="toolbar" aria-label="Render">
        <strong>Render</strong>
        <span className="render-samples">{samples < 0 ? "Preparing the scene…" : `${samples} samples`}</span>
        <select value={props.backdrop} onChange={(e) => props.onBackdrop(e.target.value as Backdrop)} aria-label="Backdrop" title="What the model stands in: a grey studio, or a seamless white sweep as product photographs are shot">
          <option value="studio">Studio</option>
          <option value="white">White sweep</option>
        </select>
        <select value={size} onChange={(e) => setSize(Number(e.target.value))} aria-label="Image size" title="Size of the saved PNG">
          {SIZES.map((s, i) => (
            <option key={s.label} value={i}>
              {s.label}
            </option>
          ))}
        </select>
        <select value={quality} onChange={(e) => setQuality(Number(e.target.value))} aria-label="Samples" title="Samples per pixel for the saved PNG: more is smoother and slower">
          {[64, 128, 256, 512, 1024].map((q) => (
            <option key={q} value={q}>
              {q} samples
            </option>
          ))}
        </select>
        <button type="button" className="primary" title="Render the picture at the chosen size and samples, and download it as a PNG" onClick={save} disabled={saving !== null}>
          {saving === null ? "Save PNG" : `Rendering ${saving}/${quality}…`}
        </button>
        <button type="button" className="secondary" title="Leave the render and go back to the model" onClick={props.onClose}>
          Close
        </button>
        {error && <span className="render-error">{error}</span>}
      </div>
    </div>
  );
}
