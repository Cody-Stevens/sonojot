// Hardware profiles for the light Speech page review (November 13, 2026).
// The catalog mirrors the ids, backends and download sizes the installed
// Windows app reports; availability and recommendations are fixture choices.
import { macProfileFixture } from "./desktop-fixture.js";
import { qwen, cpu, model, pc } from "./speech-fixture.js";

export const ft2 = "parakeet-tdt-0.6b-v2-sonojot-ft2";
export const q4 = "qwen3-asr-1.7b-q4-metal";
export const q8 = "qwen3-asr-1.7b-q8-metal";
const FT2_REASON =
  "SonoJot Parakeet is tuned by SonoJot for English dictation and meetings. It is smaller and faster than Qwen and finishes moments after you stop speaking.";

const cpuModel = (id, name, diskBytes, backend, patch = {}) => ({
  ...model(id, name, 1.5e9),
  diskBytes,
  totalBytes: diskBytes,
  downloadedBytes: 0,
  state: "available",
  backend,
  removable: false,
  ...patch,
});

const sonojot = (installedNow) =>
  cpuModel(ft2, "SonoJot Parakeet", 661_190_591, "sherpa-onnx-cpu", {
    family: "Parakeet",
    languages: ["en"],
    quantization: "int8",
    ...(installedNow
      ? { state: "installed", downloadedBytes: 661_190_591 }
      : {}),
  });

/** Windows catalog entries with measured TED-12 results and waits. */
const windowsExtras = () => [
  cpuModel(
    "qwen3-asr-0.6b-q8-cpu",
    "Qwen3-ASR 0.6B · CPU",
    1_019_141_728,
    "llama.cpp CPU",
  ),
  cpuModel(
    "moonshine-small",
    "Moonshine Small Streaming",
    142_300_974,
    "moonshine-ort-cpu",
    { state: "installed", downloadedBytes: 142_300_974 },
  ),
  cpuModel(
    "moonshine-medium",
    "Moonshine Medium Streaming",
    269_141_623,
    "moonshine-ort-cpu",
    { state: "installed", downloadedBytes: 269_141_623 },
  ),
  cpuModel("whisper-tiny.en", "Whisper Tiny (English)", 103_627_191, "CPU"),
  cpuModel("whisper-base.en", "Whisper Base (English)", 160_626_066, "CPU"),
  cpuModel("whisper-small.en", "Whisper Small (English)", 375_501_079, "CPU"),
  // Installed but never measured: belongs in the table as "not measured".
  cpuModel(
    "whisper-large-v3-turbo",
    "Whisper Large v3 Turbo",
    1_036_613_791,
    "CPU",
    { state: "installed", downloadedBytes: 1_036_613_791 },
  ),
];

function english(fixture, { dictation, other, firstRun, extras = [] }) {
  const recommendations = fixture.hardwareProfile.recommendations;
  const balanced = {
    ...recommendations.balanced,
    asrModel: ft2,
    reasons: [FT2_REASON],
  };
  return {
    ...fixture,
    firstRun,
    settings: firstRun
      ? { ...fixture.settings, setupProfile: null }
      : { ...fixture.settings, setupProfile: "balanced", asrModel: ft2 },
    libraryModels: [
      ...fixture.libraryModels,
      ...extras,
      sonojot(!firstRun),
    ],
    hardwareProfile: {
      ...fixture.hardwareProfile,
      recommendations: { ...recommendations, balanced },
      frontier: {
        dictation,
        meetings: fixture.hardwareProfile.frontier.meetings.includes(
          "granite-speech-4.1-2b-nar-q8",
        )
          ? [...dictation, "granite-speech-4.1-2b-nar-q8"]
          : dictation,
        otherLanguages: other,
      },
    },
  };
}

/** NVIDIA, CPU-only and Apple Silicon (16 GB), English, SonoJot Parakeet default. */
export function speechProfiles(firstRun = false) {
  const mac = macProfileFixture(16);
  return {
    nvidia: english(pc(), {
      dictation: [ft2, qwen],
      other: qwen,
      firstRun,
      extras: windowsExtras(),
    }),
    cpu: english(pc(false), {
      dictation: [ft2, cpu],
      other: cpu,
      firstRun,
      extras: windowsExtras(),
    }),
    mac: english(
      { ...mac, settings: { ...mac.settings, localProofreader: false } },
      { dictation: [ft2, q4, q8], other: q4, firstRun },
    ),
  };
}
