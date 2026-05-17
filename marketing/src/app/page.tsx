import Link from 'next/link';
import { ArrowRight, ShieldCheck, Cpu, Gauge } from 'lucide-react';
import { DownloadCta } from '@/components/download-cta';
import { FeatureGrid } from '@/components/feature-grid';
import { ConnectorGrid } from '@/components/connector-grid';
import { ComparisonTable } from '@/components/comparison-table';
import { PricingCards } from '@/components/pricing-cards';

export default function HomePage() {
  return (
    <>
      {/* Hero */}
      <section className="marketing-section pb-12">
        <div className="container-wide">
          <div className="max-w-3xl">
            <span className="pill mb-6">
              <ShieldCheck size={12} /> Source-available · Local-first · No telemetry
            </span>
            <h1 className="text-5xl md:text-6xl font-bold tracking-tight text-foreground leading-[1.05]">
              The file manager built for people who move data for a living.
            </h1>
            <p className="mt-6 text-xl text-foreground-secondary leading-relaxed">
              Local + remote browsing, 17 protocols, crash-safe transfers, real-time sync, and a local AI assistant
              — all in one cross-platform desktop app. Free for individuals. Always.
            </p>
            <div className="mt-8">
              <DownloadCta size="lg" />
            </div>
            <p className="mt-4 text-sm text-foreground-tertiary">
              macOS 10.15+ · Windows 10+ · Linux (.deb / .rpm / .AppImage) · 64-bit
            </p>
          </div>
        </div>
      </section>

      {/* Stat strip */}
      <section className="border-y border-border bg-background-secondary py-8">
        <div className="container-wide grid grid-cols-1 md:grid-cols-3 gap-6">
          <div className="flex items-start gap-3">
            <Cpu className="text-primary mt-1 shrink-0" size={20} />
            <div>
              <p className="font-semibold text-foreground">Rust-native engine</p>
              <p className="text-sm text-foreground-secondary">Three-layer transfer architecture: worker pool, integrity, recovery.</p>
            </div>
          </div>
          <div className="flex items-start gap-3">
            <Gauge className="text-primary mt-1 shrink-0" size={20} />
            <div>
              <p className="font-semibold text-foreground">17 protocols, one engine</p>
              <p className="text-sm text-foreground-secondary">Resume, checksum, throttle, and recover the same way across every backend.</p>
            </div>
          </div>
          <div className="flex items-start gap-3">
            <ShieldCheck className="text-primary mt-1 shrink-0" size={20} />
            <div>
              <p className="font-semibold text-foreground">Your files stay yours</p>
              <p className="text-sm text-foreground-secondary">No cloud relay. No telemetry by default. Credentials live in your OS keychain.</p>
            </div>
          </div>
        </div>
      </section>

      <FeatureGrid />
      <ConnectorGrid />
      <ComparisonTable />

      {/* Pricing teaser */}
      <section className="marketing-section">
        <div className="container-wide">
          <div className="text-center max-w-3xl mx-auto mb-10">
            <h2 className="marketing-heading">Simple pricing. Free where it should be free.</h2>
            <p className="marketing-subheading mt-4">
              The desktop app is free forever for individuals. We only charge teams for the coordination layer.
            </p>
          </div>
          <PricingCards variant="teaser" />
          <div className="mt-8 text-center">
            <Link href="/pricing" className="inline-flex items-center gap-2 text-sm font-medium">
              Compare full plans <ArrowRight size={16} />
            </Link>
          </div>
        </div>
      </section>

      {/* Final CTA */}
      <section className="marketing-section pt-0">
        <div className="container-page">
          <div className="rounded-2xl bg-primary text-primary-foreground p-12 text-center">
            <h2 className="text-3xl md:text-4xl font-bold tracking-tight">Get going in 30 seconds.</h2>
            <p className="mt-3 text-primary-foreground/90 max-w-xl mx-auto">
              Install signed, auto-updating builds for your OS. No account required. Free forever for individuals.
            </p>
            <div className="mt-8 flex flex-col sm:flex-row items-center justify-center gap-3">
              <Link
                href="/download"
                className="inline-flex items-center gap-2 px-6 py-3 rounded-md bg-background text-foreground font-medium hover:bg-background-secondary transition-colors"
              >
                Download free
              </Link>
              <Link
                href="/docs"
                className="inline-flex items-center gap-2 px-6 py-3 rounded-md border border-primary-foreground/30 text-primary-foreground font-medium hover:bg-primary-hover transition-colors"
              >
                Read the docs
              </Link>
            </div>
          </div>
        </div>
      </section>
    </>
  );
}
