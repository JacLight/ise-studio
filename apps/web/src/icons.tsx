// Line icons from the design mockup (1.65px stroke, 24px grid).
const PATHS = {
  cube: "m12 2 9 5v10l-9 5-9-5V7z M3 7l9 5 9-5 M12 12v10",
  spark: "m12 3 2.5 6.5L21 12l-6.5 2.5L12 21l-2.5-6.5L3 12l6.5-2.5z M20 2v4 M18 4h4",
  share: "M8 12h12 M15 7l5 5-5 5 M12 4H5v16h7",
  chevron: "m9 5 7 7-7 7",
  down: "m6 9 6 6 6-6",
  plus: "M12 5v14 M5 12h14",
  minus: "M5 12h14",
  comment: "M4 4h16v13H9l-5 4z",
  check: "m5 12 4 4L19 6",
  arrow: "M5 19 19 5 M9 5h10v10",
  history: "M3 10a9 9 0 1 1 1 8 M3 4v6h6 M12 7v6l4 2",
  cursor: "m5 3 14 10-7 1-3 7z",
  undo: "M8 5 3 10l5 5 M3 10h11a6 6 0 0 1 6 6",
  redo: "m16 5 5 5-5 5 M21 10H10a6 6 0 0 0-6 6",
  expand: "M8 3H3v5 M16 3h5v5 M3 16v5h5 M21 16v5h-5",
  cloud: "M6 18a5 5 0 0 1-1-10 7 7 0 0 1 13-1 6 6 0 0 1 0 11z",
  eye: "M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12 M15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0",
  layers: "m12 3 10 5-10 5L2 8z M2 12l10 5 10-5 M2 17l10 5 10-5",
  sliders: "M4 5h16 M4 12h16 M4 19h16 M8 3v4 M16 10v4 M10 17v4",
  rotate: "M4 8a9 9 0 1 1-1 8 M4 3v5h5",
  close: "M6 6l12 12 M6 18 18 6",
  send: "M12 18V5 M7 10l5-5 5 5",
  download: "M12 3v12 M7 10l5 5 5-5 M4 16v5h16v-5",
  grid: "M3 3h18v18H3z M3 9h18 M3 15h18 M9 3v18 M15 3v18",
  dots: "M5 12h.01 M12 12h.01 M19 12h.01",
  dock: "M3 4h18v16H3z M14 4v16",
  mic: "M9 5a3 3 0 0 1 6 0v7a3 3 0 0 1-6 0z M6 10v2a6 6 0 0 0 12 0v-2 M12 18v4 M8 22h8",
  trash: "M4 7h16 M10 11v6 M14 11v6 M6 7l1 13h10l1-13 M9 7V4h6v3",
  fit: "M4 9V4h5 M15 4h5v5 M20 15v5h-5 M9 20H4v-5 M9 9h6v6H9z",
  link: "m9 15 6-6 M8 16l-2 2a4 4 0 0 1-5-5l5-5a4 4 0 0 1 5 0 M16 8l2-2a4 4 0 0 1 5 5l-5 5a4 4 0 0 1-5 0",
  alert: "M12 3 2 20h20z M12 10v4 M12 17h.01",
  shield: "m12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6z M8 12l3 3 5-6",
  weight: "M5 9h14l2 11H3z M9 9a3 3 0 1 1 6 0",
  envelope: "M3 8l9-5 9 5v8l-9 5-9-5z M3 8l9 5 9-5 M12 13v8",
  ruler: "m3 16 13-13 5 5L8 21z M12 7l2 2 M8 11l2 2 M4 15l2 2",
  collide: "M3 8h10v10H3z M11 4h10v10H11z",
  explode: "M10 10h4v4h-4z M4 4l4 4 M4 7.5V4h3.5 M20 4l-4 4 M16.5 4H20v3.5 M4 20l4-4 M4 16.5V20h3.5 M20 20l-4-4 M20 16.5V20h-3.5",
  refresh: "M20 11a8 8 0 1 0-2.3 5.7 M20 4v7h-7",
  pencil: "M4 20l1-4L16 5l3 3L8 19z M14 7l3 3",
  mail: "M3 6h18v12H3z M3 7l9 6 9-6",
  printer: "M7 8V3h10v5 M5 8h14a2 2 0 0 1 2 2v6h-4 M7 16H3v-6a2 2 0 0 1 2-2 M7 13h10v8H7z",
  fillet: "M4 21V13a9 9 0 0 1 9-9h8 M4 4h4 M4 4v4",
  shell: "M3 7v13h18V7 M7 7v9h10V7 M3 7h4 M17 7h4",
  draft: "M4 21h16 M6.5 21 9 9h6l2.5 12 M12 7V2 M9.5 4.5 12 2l2.5 2.5",
  play: "M7 4.5v15l12-7.5z",
  pause: "M8 5v14 M16 5v14",
  rotateCw: "M20 8a9 9 0 1 0 1 8 M20 3v5h-5",
  flipH: "M12 3v18 M9 7 4 17h5z M15 7l5 10h-5z",
  flipV: "M3 12h18 M7 9 17 4v5z M7 15l10 5v-5z",
  alignLeft: "M4 3v18 M8 7h11v4H8z M8 13h6v4H8z",
  alignHCenter: "M12 3v18 M6 7h12v4H6z M8 13h8v4H8z",
  alignRight: "M20 3v18 M5 7h11v4H5z M10 13h6v4h-6z",
  alignTop: "M3 4h18 M7 8h4v11H7z M13 8h4v6h-4z",
  alignVCenter: "M3 12h18 M7 6h4v12H7z M13 8h4v8h-4z",
  alignBottom: "M3 20h18 M7 5h4v11H7z M13 10h4v6h-4z",
  distH: "M4 3v18 M20 3v18 M10 7h4v10h-4z",
  distV: "M3 4h18 M3 20h18 M7 10h10v4H7z",
  copy: "M8 8h12v12H8z M4 16V4h12",
  scale: "M4 20 20 4 M14 4h6v6 M4 14v6h6",
  unite: "M4 4h10v6h6v10H10v-6H4z",
  subtract: "M4 4h10v6h-4v4H4z M13 13h7v7h-7z",
  intersect: "M10 10h4v4h-4z M4 14V4h10 M20 10v10H10",
  offset: "M8 8h8v8H8z M3 3h18v18H3z",
  patternLinear: "M3 9h4v6H3z M10 9h4v6h-4z M17 9h4v6h-4z",
  patternCircular: "M12 2.5a2 2 0 1 0 0 4 2 2 0 0 0 0-4z M19.5 15a2 2 0 1 0 0 4 2 2 0 0 0 0-4z M4.5 15a2 2 0 1 0 0 4 2 2 0 0 0 0-4z M12 10.5a1.5 1.5 0 1 0 0 3 1.5 1.5 0 0 0 0-3z",
  mirror: "M12 3v18 M9 6 3 18h6z M15 6v12h6z",
  project: "M4 8l8-4 8 4-8 4z M12 12v3 M4 20h16 M7 17.5 12 15l5 2.5",
  // Sketch constraints.
  cHorizontal: "M4 12h16 M4 9v6 M20 9v6",
  cVertical: "M12 4v16 M9 4h6 M9 20h6",
  cParallel: "M6 20 14 4 M10 20l8-16",
  cPerpendicular: "M5 19h14 M12 19V5",
  cEqual: "M5 9h14 M5 15h14",
  cTangent: "M3 19h18 M12 19a6 6 0 1 0 0-12 6 6 0 0 0 0 12z",
  cConcentric: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18z M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8z",
  cCoincident: "M4 20 12 12 M20 20l-8-8 M12 10.5a1.5 1.5 0 1 0 0 3 1.5 1.5 0 0 0 0-3z",
  cOnCurve: "M3 17c4-9 14-9 18 0 M12 8.5a1.8 1.8 0 1 0 0 3.6 1.8 1.8 0 0 0 0-3.6z",
  cMidpoint: "M3 12h18 M12 9.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5z M3 9v6 M21 9v6",
  cFix: "M12 13v8 M8 3h8 M9 3v6l-3 4h12l-3-4V3",
} as const;

export type IconName = keyof typeof PATHS;

export function Icon({ name }: { name: IconName }) {
  return (
    <svg className="icon" viewBox="0 0 24 24" aria-hidden="true">
      <path d={PATHS[name]} />
    </svg>
  );
}

/** Icon-only button, the mockup's `.ib`. */
export function IconButton(props: {
  icon: IconName;
  label: string;
  onClick: () => void;
  className?: string;
  pressed?: boolean;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      className={`ib ${props.className ?? ""}`}
      onClick={props.onClick}
      title={props.label}
      aria-label={props.label}
      aria-pressed={props.pressed}
      disabled={props.disabled}
    >
      <Icon name={props.icon} />
    </button>
  );
}
