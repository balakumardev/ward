/* @refresh reload */
import "./styles/tokens.css";
import "./styles/app.css";
import { render } from "solid-js/web";
import App from "./App";

async function boot() {
  // Dev-only: when launched via `npm run dev:mock`, install the mock Tauri
  // bridge BEFORE mounting so `isTauri()` is true and `invoke` is wired to
  // the mock store. Absent the flag (native `tauri dev`, production builds)
  // this branch is dead and `./mock/*` is never fetched.
  if (import.meta.env.VITE_WARD_MOCK) {
    await import("./mock/install");
  }
  render(() => <App />, document.getElementById("root") as HTMLElement);
}

void boot();
