/** Small renderer for the bundled legal text. Raw HTML is always text. */
function escape(text: string): string {
  return text.replace(
    /[&<>"']/g,
    (character) =>
      ({
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        '"': "&quot;",
        "'": "&#39;",
      })[character] ?? character,
  );
}

/** HTTPS only, like the desktop opener; anything else stays plain text. */
export function legalLink(value: string): string | null {
  try {
    const url = new URL(value);
    return url.protocol === "https:" && !url.username && !url.password
      ? url.href
      : null;
  } catch {
    return null;
  }
}

function inline(text: string): string {
  const token = /\[([^\]\n]+)\]\(([^\s)]+)\)|\*\*([^\n]+?)\*\*|`([^`\n]+)`/g;
  let html = "",
    end = 0;
  for (const match of text.matchAll(token)) {
    html += escape(text.slice(end, match.index));
    if (match[1] !== undefined) {
      const href = legalLink(match[2]);
      html += href
        ? `<a href="${escape(href)}" target="_blank" rel="noopener noreferrer">${escape(match[1])}</a>`
        : escape(match[0]);
    } else if (match[3] !== undefined)
      html += `<strong>${inline(match[3])}</strong>`;
    else html += `<code>${escape(match[4])}</code>`;
    end = match.index + match[0].length;
  }
  return html + escape(text.slice(end));
}

export function renderLegalMarkdown(markdown: string): string {
  const lines = markdown.replace(/\r\n?/g, "\n").split("\n");
  const result: string[] = [];
  let paragraph: string[] = [],
    list: "ul" | "ol" | null = null;
  const flush = () => {
    if (paragraph.length) result.push(`<p>${inline(paragraph.join(" "))}</p>`);
    paragraph = [];
  };
  const closeList = () => {
    if (list) result.push(`</${list}>`);
    list = null;
  };
  for (const line of lines) {
    const heading = /^(#{1,6})\s+(.+)$/.exec(line);
    const item = /^\s*(?:([-*])|\d+\.)\s+(.+)$/.exec(line);
    if (heading) {
      flush();
      closeList();
      const level = Math.min(heading[1].length + 1, 6);
      result.push(
        `<h${level} data-text-surface="emboss">${inline(heading[2])}</h${level}>`,
      );
    } else if (item) {
      flush();
      const kind = item[1] ? "ul" : "ol";
      if (list !== kind) {
        closeList();
        result.push(`<${kind}>`);
        list = kind;
      }
      result.push(`<li>${inline(item[2])}</li>`);
    } else if (!line.trim()) {
      flush();
      closeList();
    } else {
      closeList();
      paragraph.push(line.trim());
    }
  }
  flush();
  closeList();
  return result.join("\n");
}
