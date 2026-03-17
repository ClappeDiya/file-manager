/**
 * Environment configuration for the admin console.
 *
 * All runtime configuration is sourced from environment variables.
 * In production, these are set via the deployment platform (Vercel, Docker, etc.).
 * In development, create a .env.local file from .env.example.
 */

/** Server-side environment variables (never exposed to client) */
export const serverEnv = {
  /** PostgreSQL connection string */
  databaseUrl: process.env.DATABASE_URL ?? "",
  /** NextAuth.js secret for JWT signing */
  nextAuthSecret: process.env.NEXTAUTH_SECRET ?? "",
  /** NextAuth.js callback URL */
  nextAuthUrl: process.env.NEXTAUTH_URL ?? "http://localhost:3001",
  /** Google OAuth provider */
  googleClientId: process.env.GOOGLE_CLIENT_ID ?? "",
  googleClientSecret: process.env.GOOGLE_CLIENT_SECRET ?? "",
  /** GitHub OAuth provider */
  githubClientId: process.env.GITHUB_CLIENT_ID ?? "",
  githubClientSecret: process.env.GITHUB_CLIENT_SECRET ?? "",
  /** Admin API endpoint */
  adminApiUrl: process.env.ADMIN_API_URL ?? "",
  adminApiKey: process.env.ADMIN_API_KEY ?? "",
};

/** Client-side environment variables (prefixed with NEXT_PUBLIC_) */
export const clientEnv = {
  /** Public app name */
  appName: process.env.NEXT_PUBLIC_APP_NAME ?? "UFOP Admin Console",
  /** Public API URL for client-side fetching */
  publicApiUrl: process.env.NEXT_PUBLIC_API_URL ?? "",
};
