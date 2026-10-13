import { site, platforms } from "./site.config.mjs";

/**
 * The published version, read from the release host at build time so the
 * structured data never carries a hand-typed number. A failed or slow lookup
 * leaves the version out rather than failing the build.
 */
export async function latestVersion() {
  try {
    const response = await fetch(site.latestRelease, { signal: AbortSignal.timeout(5000) });
    if (!response.ok) return undefined;
    const { version } = await response.json();
    return typeof version === "string" && /^\d+\.\d+\.\d+/.test(version) ? version : undefined;
  } catch {
    return undefined;
  }
}

/** JSON-LD for the home page: the publisher, the website and the app. */
export function homeStructuredData(version) {
  const home = new URL("/", site.origin).href;
  const publisher = { "@id": `${home}#publisher` };
  return {
    "@context": "https://schema.org",
    "@graph": [
      {
        "@type": "Organization",
        "@id": publisher["@id"],
        name: "Cody Stevens Studio LLC",
        url: "https://codystevens.studio",
      },
      {
        "@type": "WebSite",
        "@id": `${home}#website`,
        url: home,
        name: site.name,
        inLanguage: "en",
        publisher,
      },
      {
        "@type": "SoftwareApplication",
        "@id": `${home}#app`,
        name: site.name,
        description:
          "Free, open-source dictation and meeting transcription for Windows and macOS. Speech recognition runs on your own computer.",
        url: home,
        image: new URL("/brand/app-icon-512.png", site.origin).href,
        screenshot: new URL("/shots/app-dictation-demo-fog.webp", site.origin).href,
        operatingSystem: "Windows, macOS",
        applicationCategory: "UtilitiesApplication",
        applicationSubCategory: "Dictation and speech to text",
        ...(version ? { softwareVersion: version } : {}),
        downloadUrl: platforms
          .filter((platform) => platform.status === "released")
          .map((platform) => site.asset(platform.asset)),
        offers: { "@type": "Offer", price: "0", priceCurrency: "USD" },
        isAccessibleForFree: true,
        license: "https://www.apache.org/licenses/LICENSE-2.0",
        inLanguage: "en",
        author: publisher,
        publisher,
      },
    ],
  };
}

/** Serialises JSON-LD for a script element without letting it close the tag. */
export const jsonLd = (data) => JSON.stringify(data).replace(/</g, "\\u003c");
