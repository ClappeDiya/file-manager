import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { createSlugger } from '@/lib/docs-search';
import { resolveDocHref } from '@/lib/docs';

type HastNode = { type: string; tagName?: string; value?: string; properties?: Record<string, unknown>; children?: HastNode[] };

const textOf = (n: HastNode): string => (n.type === 'text' ? (n.value ?? '') : (n.children ?? []).map(textOf).join(''));

/**
 * Gives every heading an `id` (GitHub-style, repeated headings numbered) so search results can link to
 * it. Uses the same slugger as the search indexer, so an index anchor always exists on the page.
 * Scrollable code samples receive distinct landmark names within each document.
 */
export function rehypeDocumentSemantics() {
  return (tree: HastNode) => {
    const slug = createSlugger();
    let codeSample = 0;
    const walk = (n: HastNode) => {
      if (n.type === 'element' && n.tagName && /^h[1-6]$/.test(n.tagName)) {
        n.properties = { ...n.properties, id: slug(textOf(n).replace(/\s+/g, ' ').trim()) };
      }
      if (n.type === 'element' && n.tagName === 'pre') {
        n.properties = { ...n.properties, tabIndex: 0, role: 'region', ariaLabel: `Code sample ${++codeSample}` };
      }
      n.children?.forEach(walk);
    };
    walk(tree);
  };
}

/** A published document's Markdown, rendered with heading anchors and links rewritten for the site. */
export function DocMarkdown({ content }: { content: string }) {
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      rehypePlugins={[rehypeDocumentSemantics]}
      components={{
        a: ({ node: _node, href, ...rest }) => <a {...rest} href={href === undefined ? undefined : resolveDocHref(href)} />,
      }}
    >
      {content}
    </ReactMarkdown>
  );
}
