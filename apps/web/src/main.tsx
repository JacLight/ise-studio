import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { Engine } from "./engine";
import { JoinScreen } from "./Team";
import "./styles.css";

const root = createRoot(document.getElementById("root")!);
if (location.pathname === "/join") {
  // An invite link: join the project on this computer, then open it.
  root.render(
    <StrictMode>
      <JoinScreen />
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
