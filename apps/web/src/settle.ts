// A view motion asked for (an exploded view playing, a shape's pieces easing
// apart): a render asked for meanwhile waits for it to end, so the picture
// shows the parts where they come to rest, not part way; and a camera asked
// for meanwhile (./ise view camera, frame) is asked again once they are there,
// instead of being framed on where they were.

export type Motion = { done: Promise<void>; end: () => void; camera?: () => void };

let current: Motion | null = null;

/** Starts a motion (ending one under way). It ends by itself after 8 s whatever happens, never holding a render up for long. */
export function beginMotion(): Motion {
  current?.end();
  let finish = () => {};
  const done = new Promise<void>((r) => (finish = r));
  const m: Motion = { done, end: () => (current === m && (current = null), finish()) };
  current = m;
  window.setTimeout(m.end, 8000);
  return m;
}

/** The motion under way, if any. */
export const motionNow = () => current;

/** Ends a motion once what it moved is drawn and a framing it asked for has begun. */
export function endMotion(m: Motion | null | undefined) {
  if (m) window.setTimeout(() => requestAnimationFrame(m.end), 150);
}

/** Resolves once no motion is under way (at once if none is). */
export const settled = () => current?.done ?? Promise.resolve();

/** The camera was set by name or numbers (`again` sets it once more): a motion under way ends framed that way. */
export function cameraAsked(again: () => void) {
  if (current) current.camera = again;
}

/** Frames the end of a motion: as a camera asked for while it ran (once the last poses are drawn), else `usual`. */
export function frameEnd(m: Motion | null | undefined, usual: () => void) {
  if (m?.camera) window.setTimeout(m.camera, 80);
  else usual();
}
