import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { Engine, projectUrl } from "./engine";
import { JoinScreen } from "./Team";
import { Viewer } from "./Viewer";
import "./styles.css";

const root = createRoot(document.getElementById("root")!);
const params = new URLSearchParams(location.search);
const link = /^\/view\/([0-9a-f]{32,128})\/?$/.exec(location.pathname);
if (location.pathname === "/join") {
  // An invite link: join the project on this computer, then open it.
  root.render(
    <StrictMode>
      <JoinScreen />
    </StrictMode>,
  );
} else if (link) {
  // A view-only link: the model, read-only, for someone who may not have Forma.
  const engine = new Engine(`/view/${link[1]}/ws`);
  engine.connect();
  root.render(
    <StrictMode>
      <Viewer engine={engine} />
    </StrictMode>,
  );
} else if (params.has("version") || params.has("at")) {
  // A past version of the project, read-only.
  const which = params.has("version") ? `version=${encodeURIComponent(params.get("version")!)}` : `at=${encodeURIComponent(params.get("at")!)}`;
  const engine = new Engine(projectUrl(`/ws?${which}`));
  engine.connect();
  root.render(
    <StrictMode>
      <Viewer engine={engine} />
    </StrictMode>,
  );
} else {
  const engine = new Engine();
  engine.connect();
  root.render(
    <StrictMode>
      <App engine={engine} />
    </StrictMode>,
  );
}
