/** Next.js server start hook: run background jobs in the Node.js runtime only. */
export async function register() {
  if (process.env.NEXT_RUNTIME === 'nodejs') {
    const { startJobs } = await import('./lib/jobs');
    startJobs();
  }
}
