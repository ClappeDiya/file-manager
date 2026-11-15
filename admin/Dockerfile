# syntax=docker/dockerfile:1.7
FROM node:22-bookworm-slim AS deps
RUN apt-get update && apt-get install -y --no-install-recommends python3 make g++ ca-certificates && rm -rf /var/lib/apt/lists/*
RUN corepack enable && corepack prepare pnpm@10.27.0 --activate
WORKDIR /repo
COPY pnpm-workspace.yaml pnpm-lock.yaml package.json .npmrc .pnpmfile.cjs ./
COPY admin/package.json admin/
COPY packages/design-tokens/package.json packages/design-tokens/
COPY packages/ui-components/package.json packages/ui-components/
RUN pnpm install --frozen-lockfile --filter admin... --prod=false
FROM deps AS build
COPY packages packages
COPY admin admin
WORKDIR /repo/admin
ENV NEXT_TELEMETRY_DISABLED=1
RUN pnpm build
FROM node:22-bookworm-slim AS runner
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* && mkdir -p /data && chown node:node /data
WORKDIR /app
ENV NODE_ENV=production NEXT_TELEMETRY_DISABLED=1 PORT=3001 HOSTNAME=0.0.0.0 ADMIN_DB_PATH=/data/admin.sqlite
COPY --from=build --chown=node:node /repo/admin/.next/standalone ./
COPY --from=build --chown=node:node /repo/admin/.next/static ./admin/.next/static
USER node
EXPOSE 3001
CMD ["node", "admin/server.js"]
