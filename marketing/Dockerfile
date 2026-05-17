# syntax=docker/dockerfile:1.7

# Marketing site for filemanager.clappe.com
# Multi-stage build for Dokploy + GHCR pipeline.

FROM node:22-alpine AS base
RUN corepack enable && corepack prepare pnpm@10.27.0 --activate
WORKDIR /repo

# ---------------------------------------------------------------------------
# deps: install workspace deps once, cache across stages
# ---------------------------------------------------------------------------
FROM base AS deps
COPY pnpm-workspace.yaml pnpm-lock.yaml package.json ./
COPY packages/design-tokens/package.json packages/design-tokens/
COPY packages/ui-components/package.json packages/ui-components/
COPY marketing/package.json marketing/
RUN pnpm install --frozen-lockfile --filter marketing... --prod=false

# ---------------------------------------------------------------------------
# build: produce .next/standalone output
# ---------------------------------------------------------------------------
FROM deps AS build
COPY packages/design-tokens packages/design-tokens
COPY packages/ui-components packages/ui-components
COPY marketing marketing
WORKDIR /repo/marketing
ENV NEXT_TELEMETRY_DISABLED=1
RUN pnpm build

# ---------------------------------------------------------------------------
# runner: minimal image for production
# ---------------------------------------------------------------------------
FROM node:22-alpine AS runner
WORKDIR /app
ENV NODE_ENV=production
ENV NEXT_TELEMETRY_DISABLED=1
ENV PORT=3002
ENV HOSTNAME=0.0.0.0

RUN addgroup -S app && adduser -S app -G app

COPY --from=build --chown=app:app /repo/marketing/.next/standalone ./
COPY --from=build --chown=app:app /repo/marketing/.next/static ./marketing/.next/static
COPY --from=build --chown=app:app /repo/marketing/public ./marketing/public

USER app
EXPOSE 3002

CMD ["node", "marketing/server.js"]
