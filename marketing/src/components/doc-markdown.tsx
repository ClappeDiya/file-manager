import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { createSlugger } from '@/lib/docs-search';

type HastNode = { type: string; tagName?: string; value?: string; properties?: Record<string, unknown>; children?: HastNode[] };

const textOf = (n: HastNode): string => (n.type === 'text' ? (n.value ?? '') : (n.children ?? []).map(textOf).join(''));

/**
 * Gives every heading an `id` (GitHub-style, repeated headings numbered) so search results can link to
 * it. Uses the same slugger as the search indexer, so an index anchor always exists on the page.
 */
export function rehypeHeadingIds() {
  return (tree: HastNode) => {
    const slug = createSlugger();
    const walk = (n: HastNode) => {
      if (n.type === 'element' && n.tagName && /^h[1-6]$/.test(n.tagName)) {
        n.properties = { ...n.properties, id: slug(textOf(n).replace(/\s+/g, ' ').trim()) };
      }
      n.children?.forEach(walk);
    };
    walk(tree);
  };
}

/** A published document's Markdown, rendered with heading anchors. */
export function DocMarkdown({ content }: { content: string }) {
  return (
    <ReactMarkdown remarkPlugins={[remarkGfm]} rehypePlugins={[rehypeHeadingIds]}>
      {content}
    </ReactMarkdown>
  );
}
