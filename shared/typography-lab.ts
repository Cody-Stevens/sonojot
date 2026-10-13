/**
 * Developer comparison surface for the shared typography material.
 * Renders identical live-text specimens on cream and charcoal stock so the
 * treatments in typography-material.css can be compared and tuned side by
 * side. The lab only sets CSS variables on its own panels; nothing persists,
 * and no control calls into host application code.
 */

export interface TypographyLabOptions {
  /** data-theme value for the cream panel. */
  lightTheme?: string;
  /** data-theme value for the charcoal panel ("slate" in the app, "basalt" on the site). */
  darkTheme?: string;
  /** Extra class a host needs for its palette (the app passes "studio"). */
  panelClass?: string;
  lightTitle?: string;
  darkTitle?: string;
  lightNote?: string;
  darkNote?: string;
}

interface LabControl {
  property: string;
  label: string;
  min: number;
  max: number;
  step: number;
  value: number;
  unit: string;
}

const CONTROLS: LabControl[] = [
  {
    property: "--type-depth-scale",
    label: "Impression depth",
    min: 0,
    max: 2,
    step: 0.01,
    value: 1,
    unit: "",
  },
  {
    property: "--type-highlight-scale",
    label: "Highlight strength",
    min: 0,
    max: 2,
    step: 0.01,
    value: 1,
    unit: "",
  },
  {
    property: "--type-shadow-scale",
    label: "Shadow strength",
    min: 0,
    max: 2,
    step: 0.01,
    value: 1,
    unit: "",
  },
  {
    property: "--type-ink-texture-strength",
    label: "Ink texture",
    min: 0,
    max: 1,
    step: 0.01,
    value: 0.38,
    unit: "",
  },
  {
    property: "--type-paper-interaction",
    label: "Paper interaction",
    min: 0,
    max: 1,
    step: 0.005,
    value: 0.035,
    unit: "",
  },
  {
    property: "--type-edge-softness",
    label: "Edge softness",
    min: 0,
    max: 1.5,
    step: 0.01,
    value: 0.12,
    unit: "px",
  },
  {
    property: "--type-ink-density",
    label: "Ink density",
    min: 0.85,
    max: 1,
    step: 0.005,
    value: 1,
    unit: "",
  },
  {
    property: "--material-light-angle",
    label: "Light angle",
    min: 15,
    max: 75,
    step: 1,
    value: 45,
    unit: "deg",
  },
];

const SURFACES = ["flat", "imprint", "emboss"] as const;

function formatValue(control: LabControl, value: number): string {
  return `${Number(value.toFixed(3))}${control.unit}`;
}

function treatmentRow(surface: (typeof SURFACES)[number]): string {
  const tag = `<span class="type-lab-tag">${surface}</span>`;
  return `<div class="type-lab-treatment">${tag}
    <div class="type-lab-sample" ${surface === "flat" ? 'data-text-surface="flat"' : ""}>
      <p class="type-lab-line text-grain" data-text-surface="${surface}">SonoJot</p>
      <p class="type-lab-sub">Your voice, in writing. <strong>Every word stays yours.</strong></p>
    </div>
  </div>`;
}

function panelHTML(
  theme: string,
  title: string,
  note: string,
  panelClass: string,
): string {
  const rows = SURFACES.map(treatmentRow).join("");
  return `<section class="type-lab-panel stock type-material${panelClass ? ` ${panelClass}` : ""}"
    data-theme="${theme}" data-material="on" aria-label="${title} typography specimens">
    <header class="type-lab-panel-head">
      <h2 class="type-lab-panel-title">${title}</h2>
      <p class="type-lab-panel-note">${note}</p>
    </header>
    <p class="type-lab-wordmark text-grain" data-text-surface="emboss-strong">SonoJot</p>
    <div class="type-lab-treatments" aria-label="Surface treatment comparison">${rows}
      <div class="type-lab-treatment"><span class="type-lab-tag">original</span><div class="type-lab-sample" data-type-material="off" data-native-type><p class="type-lab-line">SonoJot</p><p class="type-lab-sub">Original native rendering, without material.</p></div></div>
    </div>
    <div class="type-lab-block">
      <span class="type-lab-tag">headings</span>
      <h1 class="type-lab-h1 text-grain" data-text-surface="emboss">The page holds its shape</h1>
      <h2 class="type-lab-h2">Mounted pieces and quiet edges</h2>
      <h3 class="type-lab-h3">A smaller section heading</h3>
    </div>
    <div class="type-lab-block">
      <span class="type-lab-tag">body &amp; small</span>
      <p class="type-lab-body">Body copy sits at reading size and stays live text you can select.
        It keeps its own ink on the stock underneath, without a halo, so a paragraph still
        reads as print rather than a rendering trick.</p>
      <p class="type-lab-meta-line"><small>September 12 · 10:30 AM · 126 words</small></p>
    </div>
    <div class="type-lab-block">
      <span class="type-lab-tag">labels &amp; fields</span>
      <div class="type-lab-field">
        <label class="type-lab-label" for="tl-${theme}-input">Microphone</label>
        <input class="type-lab-input" id="tl-${theme}-input" type="text" value="Studio display" />
      </div>
    </div>
    <div class="type-lab-block">
      <span class="type-lab-tag">buttons</span>
      <div class="type-lab-actions">
        <button type="button" class="type-lab-btn is-ink">Save copy</button>
        <button type="button" class="type-lab-btn">Keep as heard</button>
        <button type="button" class="type-lab-btn is-quiet">Refresh</button>
        <button type="button" class="type-lab-btn" disabled>Unavailable</button>
      </div>
    </div>
    <div class="type-lab-block">
      <span class="type-lab-tag">navigation</span>
      <nav class="type-lab-nav" aria-label="${title} specimen navigation">
        <button type="button" class="type-lab-nav-item" aria-current="page">Dictation</button>
        <button type="button" class="type-lab-nav-item">Microphone</button>
        <button type="button" class="type-lab-nav-item">Meetings</button>
        <button type="button" class="type-lab-nav-item">About</button>
      </nav>
    </div>
    <div class="type-lab-block">
      <span class="type-lab-tag">metadata</span>
      <dl class="type-lab-dl">
        <div><dt>Recorded</dt><dd>September 12, 2026</dd></div>
        <div><dt>Duration</dt><dd>1:42</dd></div>
        <div><dt>Model</dt><dd>Moonshine base</dd></div>
      </dl>
    </div>
    <div class="type-lab-block">
      <span class="type-lab-tag">code</span>
      <pre class="type-lab-code"><code>const note = await sonojot.dictate();
note.save({ cleanup: "clean" });</code></pre>
    </div>
  </section>`;
}

function controlHTML(control: LabControl, index: number): string {
  const id = `type-lab-ctl-${index}`;
  return `<div class="type-lab-control">
    <label for="${id}">${control.label} <code>${control.property}</code></label>
    <div class="type-lab-slider-row">
      <input type="range" id="${id}" data-property="${control.property}"
        min="${control.min}" max="${control.max}" step="${control.step}" value="${control.value}" />
      <output for="${id}">${formatValue(control, control.value)}</output>
    </div>
  </div>`;
}

export function mountTypographyLab(
  root: HTMLElement,
  options: TypographyLabOptions = {},
): () => void {
  const lightTheme = options.lightTheme ?? "fog";
  const darkTheme = options.darkTheme ?? "slate";
  const panelClass = options.panelClass ?? "";
  const themes = [
    {
      theme: lightTheme,
      title: options.lightTitle ?? "Fog",
      note: options.lightNote ?? "Cream stock, green ink.",
    },
    {
      theme: darkTheme,
      title: options.darkTitle ?? "Slate",
      note: options.darkNote ?? "Charcoal stock, warm ivory ink.",
    },
  ];

  const lab = document.createElement("div");
  lab.className = "type-lab";
  lab.innerHTML = `
    <header class="type-lab-head">
      <h1>Typography material</h1>
      <p>Titles and important text are raised; secondary copy is flat printed;
        everyday text is imprinted. All three carry paper fibers. Switch materials off for the original rendering.</p>
    </header>
    <div class="type-lab-controls" role="group" aria-label="Material tuning">
      <label class="type-lab-target">Tune
        <select data-tune-theme><option value="both">Both stocks</option><option value="${lightTheme}">Cream</option><option value="${darkTheme}">Charcoal</option></select>
      </label>
      ${CONTROLS.map(controlHTML).join("")}
      <label class="type-lab-toggle">
        <input type="checkbox" checked /> Type material
      </label>
      <div class="type-lab-tools">
        <button type="button" class="type-lab-tool" data-action="reset">Reset</button>
        <button type="button" class="type-lab-tool" data-action="copy">Copy CSS</button>
      </div>
      <p class="type-lab-status" role="status" aria-live="polite"></p>
    </div>
    <div class="type-lab-panels">
      ${themes.map((t) => panelHTML(t.theme, t.title, t.note, panelClass)).join("")}
    </div>`;
  root.append(lab);

  const panels = [...lab.querySelectorAll<HTMLElement>(".type-lab-panel")];
  const sliders = [
    ...lab.querySelectorAll<HTMLInputElement>('input[type="range"]'),
  ];
  const toggle = lab.querySelector<HTMLInputElement>(".type-lab-toggle input");
  const status = lab.querySelector<HTMLElement>(".type-lab-status");
  const tuneTheme = lab.querySelector<HTMLSelectElement>("[data-tune-theme]");

  function targets(): HTMLElement[] {
    return panels.filter(
      (panel) =>
        tuneTheme?.value === "both" || panel.dataset.theme === tuneTheme?.value,
    );
  }

  function syncControls(): void {
    const panel = targets()[0];
    if (!panel) return;
    const style = getComputedStyle(panel);
    for (const slider of sliders) {
      const control = controlFor(slider);
      if (!control) continue;
      const value = parseFloat(style.getPropertyValue(control.property));
      slider.value = String(Number.isFinite(value) ? value : control.value);
      const output = slider.parentElement?.querySelector("output");
      if (output) output.value = formatValue(control, Number(slider.value));
    }
  }

  const controlFor = (input: HTMLInputElement) =>
    CONTROLS.find((c) => c.property === input.dataset.property);

  function apply(input: HTMLInputElement): void {
    const control = controlFor(input);
    if (!control) return;
    const value = Number(input.value);
    for (const panel of control.property === "--material-light-angle"
      ? panels
      : targets()) {
      panel.style.setProperty(control.property, formatValue(control, value));
    }
    const output = input.parentElement?.querySelector("output");
    if (output) output.value = formatValue(control, value);
    if (status) status.textContent = "";
    document.dispatchEvent(new Event("typography-tune"));
  }

  for (const slider of sliders) {
    slider.addEventListener("input", () => apply(slider));
  }
  syncControls();
  tuneTheme?.addEventListener("change", syncControls);

  toggle?.addEventListener("change", () => {
    if (toggle.checked) lab.removeAttribute("data-type-material");
    else lab.setAttribute("data-type-material", "off");
  });

  lab.querySelector('[data-action="reset"]')?.addEventListener("click", () => {
    for (const panel of panels)
      for (const control of CONTROLS)
        panel.style.removeProperty(control.property);
    syncControls();
    document.dispatchEvent(new Event("typography-tune"));
    if (toggle && !toggle.checked) {
      toggle.checked = true;
      lab.removeAttribute("data-type-material");
    }
    if (status) status.textContent = "Reset to defaults.";
  });

  lab
    .querySelector('[data-action="copy"]')
    ?.addEventListener("click", async () => {
      const snippet = panels
        .map((panel) => {
          const style = getComputedStyle(panel);
          const lines = CONTROLS.map(
            (control) =>
              `  ${control.property}: ${style.getPropertyValue(control.property).trim()};`,
          ).join("\n");
          return `[data-theme="${panel.dataset.theme}"] {\n${lines}\n}`;
        })
        .join("\n\n");
      const text = `/* Values tuned in the typography material lab. */\n${snippet}\n`;
      try {
        await navigator.clipboard.writeText(text);
        if (status) status.textContent = "Copied CSS for both themes.";
      } catch {
        const scratch = document.createElement("textarea");
        scratch.value = text;
        scratch.setAttribute("aria-label", "Tuned typography CSS");
        scratch.className = "type-lab-copy";
        lab.append(scratch);
        scratch.select();
        const copied = document.execCommand("copy");
        if (copied) scratch.remove();
        if (status)
          status.textContent = copied
            ? "Copied CSS for both themes."
            : "Select and copy the CSS below.";
      }
    });

  return () => lab.remove();
}
