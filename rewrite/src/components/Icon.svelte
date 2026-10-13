<script lang="ts">
  export type IconName =
    | "dictation"
    | "microphone"
    | "vocabulary"
    | "meetings"
    | "appearance"
    | "eye"
    | "shortcuts"
    | "about"
    | "app"
    | "play"
    | "stop"
    | "copy"
    | "close"
    | "plus"
    | "back"
    | "folder"
    | "check"
    | "delete";
  let { name, size = 24 }: { name: IconName; size?: number } = $props();
  const paths: Record<IconName, string> = {
    app: "M4 3h16a1 1 0 0 1 1 1v16a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1ZM3 8h18M8 8v13",
    dictation: "M4 9v6m4-10v14m4-17v20m4-17v14m4-10v6",
    microphone:
      "M8 6a4 4 0 0 1 8 0v7a4 4 0 0 1-8 0V6M5 11v2a7 7 0 0 0 14 0v-2M12 20v2m-4 0h8",
    vocabulary:
      "M12 5c-3-3-6-2-9-1v16c3-1 6-2 9 1m0-16c3-3 6-2 9-1v16c-3-1-6-2-9 1V5",
    meetings:
      "M7 5H4a2 2 0 0 0-2 2v11a2 2 0 0 0 2 2h3l4 3v-3h7a2 2 0 0 0 2-2v-2M9 2h11a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2h-6l-4 3v-3H9a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2",
    appearance:
      "M2 12s4-8 10-8 10 8 10 8-4 8-10 8S2 12 2 12ZM16 12a4 4 0 1 1-8 0 4 4 0 0 1 8 0",
    eye:
      "M2 12s4-8 10-8 10 8 10 8-4 8-10 8S2 12 2 12ZM16 12a4 4 0 1 1-8 0 4 4 0 0 1 8 0",
    shortcuts:
      "M4 5h16a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2M6 9h.01M10 9h.01M14 9h.01M18 9h.01M6 13h.01M10 13h.01M14 13h.01M18 13h.01M7 16h10",
    about: "M12 8h.01M11 11h1v6m-2 0h4M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0",
    play: "m9 5 11 7-11 7V5",
    stop: "M5 5h14v14H5V5",
    copy: "M8 8h13v13H8V8M16 4V2H2v14h2",
    close: "m6 6 12 12M6 18 18 6",
    plus: "M12 4v16M4 12h16",
    back: "m10 5-7 7 7 7M3 12h18",
    folder:
      "M2 7V5a2 2 0 0 1 2-2h5l3 4h8a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V7Z",
    check: "m4 12 5 5L20 6",
    delete:
      "M4 7h16M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2m3 0-1 13a1 1 0 0 1-1 1H8a1 1 0 0 1-1-1L6 7",
  };
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.55"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"><path d={paths[name]} /></svg
>
