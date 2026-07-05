/* @refresh reload */
import "./styles/tokens.css";
import "./styles/app.css";
import { render } from "solid-js/web";
import App from "./App";
import Popover from "./entries/Popover";

/** True when this webview is the tray popover window (native label === 'popover'),
 *  or, in dev:mock browser preview, when the URL carries ?view=popover. */
function isPopoverWindow(): boolean {
  if (new URLSearchParams(window.location.search).get("view") === "popover") return true;
  const internals = (globalThis as { __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: string } } } }).__TAURI_INTERNALS__;
  return internals?.metadata?.currentWindow?.label === "popover";
}

async function boot() {
  if (import.meta.env.VITE_WARD_MOCK) {
    await import("./mock/install");
  }
  const root = document.getElementById("root") as HTMLElement;
  if (isPopoverWindow()) {
    render(() => <Popover />, root);
  } else {
    render(() => <App />, root);
  }
}

void boot();
