import { type ReactNode, useEffect, useRef } from "react";
import { IconButton } from "./icons";

export function Dialog(props: { title: string; onClose: () => void; children: ReactNode }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const d = ref.current!;
    if (!d.open) d.showModal();
    return () => d.close();
  }, []);
  return (
    <dialog ref={ref} onClose={props.onClose} onCancel={props.onClose} aria-label={props.title}>
      <header>
        <h2>{props.title}</h2>
        <IconButton icon="close" label="Close dialog" onClick={props.onClose} />
      </header>
      {props.children}
    </dialog>
  );
}

export function StorageDialog(props: { revision: number; onClose: () => void }) {
  return (
    <Dialog title="Project storage" onClose={props.onClose}>
      <p>Your project, wherever you keep it.</p>
      <label>
        Destination
        <select title="Where the project is saved" defaultValue="local">
          <option value="local">This computer (project file)</option>
          <option disabled>S3-compatible storage · not connected yet</option>
          <option disabled>DigitalOcean Spaces · not connected yet</option>
        </select>
      </label>
      <p className="demo-note">
        Every change is written to the local project file before it is confirmed. This project is at revision {props.revision}.
      </p>
      <button title="Close storage settings" type="button" className="primary" onClick={props.onClose}>
        Done
      </button>
    </Dialog>
  );
}
