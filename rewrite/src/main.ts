import { mount } from "svelte";
import App from "./App.svelte";
import "./brand/brand.css";
import "./styles.css";
import "./layout.css";
import "../../shared/typography-material.css";
import { mountTypographyMaterial } from "../../shared/typography-material";

const target = document.getElementById("app");
if (!target) throw new Error("The application root is missing.");
mount(App, { target });
// The always-present screen tab has only small type and needs no grain observer.
const view = new URLSearchParams(window.location.search).get("view");
if (view !== "overlay" && view !== "cursor") {
  const disposeMaterial = mountTypographyMaterial(target);
  import.meta.hot?.dispose(disposeMaterial);
}
