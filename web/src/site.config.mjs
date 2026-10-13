/** Stable public destinations; versioned release metadata lives on the release host. */
const repo = "Cody-Stevens/sonojot";
export const site = {
  name: "SonoJot",
  titleLine: "Free Local Dictation App for Windows and Mac",
  origin: "https://sonojot.com",
  repo,
  repoUrl: `https://github.com/${repo}`,
  issuesUrl: `https://github.com/${repo}/issues`,
  latestRelease: "https://releases.sonojot.com/latest.json",
  asset: (file) => `https://releases.sonojot.com/download/${file}`,
};
export const platforms = [
  {
    id: "windows", name: "Windows", detail: "Windows 10 and 11, 64-bit",
    status: "released", stage: "beta", asset: "SonoJot-windows-x64-setup.exe",
    command: { label: "PowerShell", lines: ["irm https://sonojot.com/install.ps1 | iex"] },
  },
  {
    id: "macos", name: "macOS", detail: "Apple Silicon, macOS 14 and later",
    status: "released", stage: "alpha", asset: "SonoJot-macos-arm64.dmg",
    command: { label: "Terminal", lines: ["curl --proto '=https' --proto-redir '=https' -fsSL https://sonojot.com/install.sh | sh"] },
  },
];
