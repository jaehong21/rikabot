import MarkdownIt from "markdown-it";
import hljs from "highlight.js";

const markdown = new MarkdownIt({
  html: false,
  breaks: true,
  linkify: true,
  highlight(code, language) {
    const hasLanguage = Boolean(language) && hljs.getLanguage(language);
    const highlighted = hasLanguage
      ? hljs.highlight(code, {
          language: language!,
          ignoreIllegals: true,
        }).value
      : hljs.highlightAuto(code).value;

    return `<pre class="hljs"><code>${highlighted}</code></pre>`;
  },
});

export function renderMarkdown(text: string): string {
  return markdown.render(text);
}
