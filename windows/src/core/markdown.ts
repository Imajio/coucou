// A small Markdown reader for model replies: headings, paragraphs, lists,
// quotes, fenced code, and inline code, bold, italic and links. It returns a
// tree; the window turns it into DOM nodes with textContent only, so nothing a
// model writes can become markup.

export type Inline =
  | { t: "text"; text: string }
  | { t: "code"; text: string }
  | { t: "strong"; children: Inline[] }
  | { t: "em"; children: Inline[] }
  | { t: "link"; href: string; children: Inline[] };

export type Block =
  | { t: "heading"; level: number; children: Inline[] }
  | { t: "paragraph"; children: Inline[] }
  | { t: "code"; lang: string; text: string }
  | { t: "list"; ordered: boolean; items: Inline[][] }
  | { t: "quote"; children: Inline[] }
  | { t: "rule" };

/** Only web and mail links are kept; anything else stays text. */
export function safeHref(href: string): string | null {
  const h = href.trim();
  return /^(https?:|mailto:)/i.test(h) ? h : null;
}

export function parseInline(src: string): Inline[] {
  const out: Inline[] = [];
  let text = "";
  const flush = () => {
    if (text) out.push({ t: "text", text });
    text = "";
  };
  let i = 0;
  while (i < src.length) {
    const c = src[i];
    if (c === "`") {
      const end = src.indexOf("`", i + 1);
      if (end > i) {
        flush();
        out.push({ t: "code", text: src.slice(i + 1, end) });
        i = end + 1;
        continue;
      }
    }
    if ((c === "*" || c === "_") && src[i + 1] === c) {
      const end = src.indexOf(c + c, i + 2);
      if (end > i + 2) {
        flush();
        out.push({ t: "strong", children: parseInline(src.slice(i + 2, end)) });
        i = end + 2;
        continue;
      }
    }
    if ((c === "*" || c === "_") && src[i + 1] !== " " && src[i + 1] !== undefined) {
      const end = src.indexOf(c, i + 1);
      // `_` inside a word (snake_case) is not emphasis.
      const wordy = c === "_" && i > 0 && /\w/.test(src[i - 1]);
      if (end > i + 1 && !wordy && src[end - 1] !== " ") {
        flush();
        out.push({ t: "em", children: parseInline(src.slice(i + 1, end)) });
        i = end + 1;
        continue;
      }
    }
    if (c === "[") {
      const close = src.indexOf("](", i + 1);
      const end = close > i ? src.indexOf(")", close + 2) : -1;
      if (close > i && end > close) {
        const href = safeHref(src.slice(close + 2, end));
        if (href) {
          flush();
          out.push({ t: "link", href, children: parseInline(src.slice(i + 1, close)) });
          i = end + 1;
          continue;
        }
      }
    }
    text += c;
    i++;
  }
  flush();
  return out;
}

const LIST_ITEM = /^\s*([-*+]|\d+[.)])\s+(.*)$/;

export function parseMarkdown(src: string): Block[] {
  const lines = src.replace(/\r\n?/g, "\n").split("\n");
  const blocks: Block[] = [];
  let para: string[] = [];
  const flushPara = () => {
    if (para.length) blocks.push({ t: "paragraph", children: parseInline(para.join("\n")) });
    para = [];
  };
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    const fence = line.match(/^\s*(```|~~~)\s*([\w+-]*)/);
    if (fence) {
      flushPara();
      const marker = fence[1];
      const body: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trim().startsWith(marker)) body.push(lines[i++]);
      i++;
      blocks.push({ t: "code", lang: fence[2] ?? "", text: body.join("\n") });
      continue;
    }
    if (!line.trim()) {
      flushPara();
      i++;
      continue;
    }
    const heading = line.match(/^(#{1,6})\s+(.*)$/);
    if (heading) {
      flushPara();
      blocks.push({ t: "heading", level: heading[1].length, children: parseInline(heading[2].replace(/\s+#+\s*$/, "")) });
      i++;
      continue;
    }
    if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(line)) {
      flushPara();
      blocks.push({ t: "rule" });
      i++;
      continue;
    }
    if (line.startsWith(">")) {
      flushPara();
      const quote: string[] = [];
      while (i < lines.length && lines[i].startsWith(">")) quote.push(lines[i++].replace(/^>\s?/, ""));
      blocks.push({ t: "quote", children: parseInline(quote.join("\n")) });
      continue;
    }
    const item = line.match(LIST_ITEM);
    if (item) {
      flushPara();
      const ordered = /\d/.test(item[1]);
      const items: Inline[][] = [];
      let current: string[] = [];
      while (i < lines.length) {
        const m = lines[i].match(LIST_ITEM);
        if (m && /\d/.test(m[1]) === ordered) {
          if (current.length) items.push(parseInline(current.join("\n")));
          current = [m[2]];
        } else if (lines[i].trim() && /^\s{2,}/.test(lines[i]) && current.length) {
          current.push(lines[i].trim());
        } else {
          break;
        }
        i++;
      }
      if (current.length) items.push(parseInline(current.join("\n")));
      blocks.push({ t: "list", ordered, items });
      continue;
    }
    para.push(line);
    i++;
  }
  flushPara();
  return blocks;
}
