import { createReliefRenderer, presets } from "./renderer";
import type { ReliefSettings } from "./renderer";

const ranges = [
  ["raise", "Raised height", 0, 6, 0.05],
  ["press", "Imprint depth", 0, 6, 0.05],
  ["bevel", "Shoulder width", 0.35, 3, 0.05],
  ["grain", "Paper micro-height", 0, 0.8, 0.02],
  ["azimuth", "Light azimuth", -180, 180, 1],
  ["elevation", "Light elevation", 12, 85, 1],
  ["ambient", "Ambient light fraction", 0.05, 0.95, 0.01],
  ["contact", "Contact shadow strength", 0, 1, 0.01],
  ["reach", "Contact reach", 0.5, 18, 0.25],
  ["pigment", "Ink coverage", 0, 1, 0.01],
  ["variation", "Fiber / ink interaction", 0, 0.5, 0.01],
] as const;

export function mountReliefLab(root: HTMLElement, host: string) {
  root.innerHTML = `<header class="proof-header"><p class="proof-kicker">SonoJot · ${host} · experimental renderer</p>
    <h1>Type formed in paper.</h1><p>One font. Three surfaces. Start with <b>Uninked</b>, then reverse the light. The recessed and raised forms should remain visible without pigment.</p></header>
    <main class="proof-main"><div class="proof-sheet" id="relief-sheet"><canvas id="relief-canvas" aria-hidden="true"></canvas>
    ${[
      [0, "Flat printing"],
      [-1, "Recessed imprint"],
      [1, "Raised emboss"],
    ]
      .map(
        (
          [mode, label],
          i,
        ) => `<section class="proof-sample" data-mode="${mode}"><p class="proof-label">0${i + 1} / ${label}</p>
      <h2 class="proof-title"><span data-ink>SonoJot</span><span class="proof-baseline" aria-hidden="true"></span></h2>
      <p class="proof-medium"><span data-ink>Your voice, in writing.</span><span class="proof-baseline" aria-hidden="true"></span></p>
      <p class="proof-body"><span data-ink>Small text stays readable.</span><span class="proof-baseline" aria-hidden="true"></span></p></section>`,
      )
      .join("")}</div>
    <aside class="proof-controls" aria-label="Material controls"><h2>Inspect the surface</h2>
    <label>Paper<select id="theme"><option value="0">Cream / pine ink</option><option value="1">Charcoal / cream ink</option></select></label>
    <label>Preset<select id="preset"><option value="diagnostic">Diagnostic · visible mechanism</option><option value="restrained">Restrained · candidate only</option></select></label>
    <label>Title size<select id="title-size"><option value="76">Wordmark · 76 px</option><option value="45">App page title · 45 px</option></select></label>
    <label>Render pass<select id="view"><option value="0">Finished material</option><option value="1">Signed height</option><option value="2">Surface normals</option><option value="3">Crisp ink coverage</option><option value="4">Signed distance</option><option value="5">Contact visibility</option><option value="6">Rounded shoulder profile</option></select></label>
    <div class="proof-actions"><button id="uninked" type="button" aria-pressed="false">Uninked</button><button id="reverse" type="button">Reverse light</button><button id="sign" type="button" aria-pressed="false">Flip relief sign</button><button id="zero" type="button">Zero relief</button></div>
    <label class="proof-check"><input id="native" type="checkbox"> Native text fallback</label>
    <details><summary>Fine controls</summary>${ranges.map(([id, label, min, max, step]) => `<label for="${id}">${label} <output id="${id}-output"></output></label><input id="${id}" type="range" min="${min}" max="${max}" step="${step}" value="${presets.diagnostic[id]}">`).join("")}</details>
    <button id="reset" type="button">Reset preset</button><p class="proof-note">Height, shoulder and reach use artistic CSS pixels. Diagnostic settings intentionally reveal the mechanism; they are not production defaults. Body relief is a diagnostic comparison only.</p>
    <output id="relief-status" class="proof-status" aria-label="Renderer status">Preparing surface…</output></aside></main>
    <footer class="proof-footer"><p>Source Serif 4, SonoJot’s existing typeface. The decorative canvas shares one stationary paper height field across all letters. Text remains in the DOM for selection and accessibility. No animation loop.</p>
    <p>This proof is limited to plain single-line Latin text. It uses procedural height, not a relit paper photograph. Production wrapping, mixed scripts and screen-reader testing remain separate acceptance gates. The existing app and website are not being migrated in this pass.</p></footer>`;
  function element<T extends HTMLElement>(id: string, kind: { new (): T }): T {
    const found = root.querySelector(`#${id}`);
    if (!(found instanceof kind)) throw new Error(`Missing ${id}`);
    return found;
  }
  const sheet = element("relief-sheet", HTMLDivElement),
    canvas = element("relief-canvas", HTMLCanvasElement),
    status = element("relief-status", HTMLOutputElement);
  const input = (id: string) => element(id, HTMLInputElement),
    select = (id: string) => element(id, HTMLSelectElement),
    button = (id: string) => element(id, HTMLButtonElement);
  const settings: ReliefSettings = {
    ...presets.diagnostic,
    dark: 0,
    blind: 0,
    view: 0,
  };
  const renderer = createReliefRenderer(sheet, canvas, (m) => {
    status.textContent =
      m.error ||
      `${m.width} × ${m.height} · backing scale ${m.scale.toFixed(2)} / requested ${m.requestedScale.toFixed(2)}\nMask + distance ${m.buildMs.toFixed(1)} ms · builds ${m.builds}\nCPU submission ${m.submitMs.toFixed(2)} ms (not GPU time)\nDraws ${m.frames} · ${(m.retainedBytes / 1048576).toFixed(1)} MiB estimated retained buffers`;
  });
  function sync() {
    root.classList.toggle("proof-dark", settings.dark === 1);
    for (const [id] of ranges) {
      input(id).value = String(settings[id]);
      element(`${id}-output`, HTMLOutputElement).value =
        `${settings[id]}${id === "azimuth" || id === "elevation" ? "°" : ""}`;
    }
    button("uninked").setAttribute(
      "aria-pressed",
      String(settings.blind === 1),
    );
    button("sign").setAttribute(
      "aria-pressed",
      String(settings.polarity === -1),
    );
    renderer.update(settings, input("native").checked);
  }
  for (const [id] of ranges)
    input(id).addEventListener("input", () => {
      settings[id] = Number(input(id).value);
      sync();
    });
  select("theme").addEventListener("change", () => {
    settings.dark = Number(select("theme").value);
    sync();
  });
  select("view").addEventListener("change", () => {
    settings.view = Number(select("view").value);
    sync();
  });
  select("title-size").addEventListener("change", () => {
    const size = select("title-size").value;
    root.style.setProperty("--proof-title-size", `${size}px`);
    root.style.setProperty(
      "--proof-title-weight",
      size === "45" ? "650" : "500",
    );
    renderer.invalidate();
  });
  function reset() {
    const preset =
      select("preset").value === "restrained"
        ? presets.restrained
        : presets.diagnostic;
    Object.assign(settings, preset, { blind: 0, view: 0 });
    select("view").value = "0";
    input("native").checked = false;
    sync();
  }
  select("preset").addEventListener("change", reset);
  button("reset").addEventListener("click", reset);
  button("uninked").addEventListener("click", () => {
    settings.blind = 1 - settings.blind;
    settings.view = 0;
    select("view").value = "0";
    sync();
  });
  button("reverse").addEventListener("click", () => {
    settings.azimuth =
      settings.azimuth >= 0 ? settings.azimuth - 180 : settings.azimuth + 180;
    sync();
  });
  button("sign").addEventListener("click", () => {
    settings.polarity *= -1;
    sync();
  });
  button("zero").addEventListener("click", () => {
    settings.raise = 0;
    settings.press = 0;
    sync();
  });
  input("native").addEventListener("change", sync);
  // Explicit diagnostics, used by the focused browser checks; never in production.
  Object.assign(window, { __reliefProof: renderer });
  sync();
  return renderer.dispose;
}
