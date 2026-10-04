// Model text as DOM: the Markdown tree from core/markdown.ts, built with
// textContent only. Links open in the browser through Coucou, never in the
// window itself.

import { h } from "../views/dom";
import { Bridge } from "../core/bridge";
import { parseMarkdown, type Block, type Inline } from "../core/markdown";

function inline(nodes: Inline[]): Node[] {
  return nodes.map((n) => {
    switch (n.t) {
      case "text":
        return document.createTextNode(n.text);
      case "code":
        return h("code", { text: n.text });
      case "strong":
        return h("strong", {}, ...inline(n.children));
      case "em":
        return h("em", {}, ...inline(n.children));
      case "link": {
        const a = h("a", { href: "#", title: n.href }, ...inline(n.children));
        a.addEventListener("click", (e) => {
          e.preventDefault();
          void Bridge.openUrl(n.href);
        });
        return a;
      }
    }
  });
}

function block(b: Block): HTMLElement {
  switch (b.t) {
    case "heading":
      return h(`h${Math.min(4, b.level + 1)}` as "h2", {}, ...inline(b.children));
    case "paragraph":
      return h("p", {}, ...inline(b.children));
    case "code": {
      const pre = h("pre", { class: "code" }, h("code", { text: b.text }));
      if (b.lang) pre.dataset.lang = b.lang;
      return pre;
    }
    case "list":
      return h(b.ordered ? "ol" : "ul", {}, ...b.items.map((item) => h("li", {}, ...inline(item))));
    case "quote":
      return h("blockquote", {}, ...inline(b.children));
    case "rule":
      return h("hr");
  }
}

export function renderMarkdown(text: string): HTMLElement {
  return h("div", { class: "md" }, ...parseMarkdown(text).map(block));
}

/** A change or a command, with added and removed lines colored. */
export function renderDetail(text: string): HTMLElement {
  const pre = h("pre", { class: "detail" });
  for (const line of text.split("\n")) {
    const cls = line.startsWith("+ ") || line === "+" ? "add" : line.startsWith("- ") || line === "-" ? "del" : "";
    pre.append(h("span", { class: cls, text: `${line}\n` }));
  }
  return pre;
}
