import { mount } from "svelte";
import App from "./App.svelte";
import TrustOverlay from "./views/TrustOverlay.svelte";
import "./brand/brand.css";
import "./styles.css";
import "./layout.css";
import "./quiet.css";
import "../../shared/typography-material.css";
import { mountTypographyMaterial } from "../../shared/typography-material";

const target = document.getElementById("app");
if (!target) throw new Error("The application root is missing.");
const view = new URLSearchParams(window.location.search).get("view");
if (view === "trust") {
  // Doubtful-word marks over another app: its own small view, no app shell.
  mount(TrustOverlay, { target });
} else {
  mount(App, { target });
}
// The always-present screen tab has only small type and needs no grain observer.
if (view !== "overlay" && view !== "cursor" && view !== "trust") {
  const disposeMaterial = mountTypographyMaterial(target);
  import.meta.hot?.dispose(disposeMaterial);
}
