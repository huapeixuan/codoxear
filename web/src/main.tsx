import { render } from "preact";
import App from "./app/App";
import "./styles/index.css";
import { installViewportCssVars } from "./lib/viewport";

installViewportCssVars();
if ("serviceWorker" in navigator) {
  window.addEventListener("load", () => {
    navigator.serviceWorker.register("service-worker.js").catch(() => undefined);
  });
}
render(<App />, document.getElementById("root")!);
