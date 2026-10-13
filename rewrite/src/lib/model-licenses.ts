import catalog from "../../licenses/model-links.json";

export interface ModelLicense {
  name: string;
  url: string;
}

const licenses = new Map<string, ModelLicense>();
for (const component of catalog.components) {
  if (component.kind !== "model") continue;
  for (const id of component.catalogIds) {
    if (licenses.has(id)) throw new Error(`Duplicate model license: ${id}`);
    licenses.set(id, {
      name: component.license,
      url:
        id === "sonojot-punct-v1" || id === "sonojot-punct-v2"
          ? "https://creativecommons.org/licenses/by-sa/4.0/legalcode.en"
          : component.licenseUrl,
    });
  }
}

/** Exact catalog IDs only: a new model must bring its own license evidence. */
export function modelLicense(id: string): ModelLicense | null {
  return licenses.get(id) ?? null;
}
