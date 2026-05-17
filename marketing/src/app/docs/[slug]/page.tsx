import type { Metadata } from 'next';
import { notFound } from 'next/navigation';
import Link from 'next/link';
import { ChevronLeft } from 'lucide-react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { listDocs, loadDoc } from '@/lib/docs';

type Params = { slug: string };

export async function generateStaticParams(): Promise<Params[]> {
  const docs = await listDocs();
  return docs.map((doc) => ({ slug: doc.slug }));
}

export async function generateMetadata({ params }: { params: Promise<Params> }): Promise<Metadata> {
  const { slug } = await params;
  const doc = await loadDoc(slug);
  if (!doc) return { title: 'Not found' };
  return {
    title: doc.entry.title,
    description: doc.entry.description,
  };
}

export default async function DocPage({ params }: { params: Promise<Params> }) {
  const { slug } = await params;
  const doc = await loadDoc(slug);
  if (!doc) notFound();

  return (
    <div className="container-prose py-16">
      <Link
        href="/docs"
        className="inline-flex items-center gap-1 text-sm text-foreground-secondary hover:text-foreground mb-6"
      >
        <ChevronLeft size={16} /> All docs
      </Link>

      <article className="prose prose-sm dark:prose-invert max-w-none">
        <ReactMarkdown remarkPlugins={[remarkGfm]}>{doc.content}</ReactMarkdown>
      </article>
    </div>
  );
}
