# ufop-server clients

Client libraries for the `ufop-server` REST API, **generated** from its OpenAPI document by `src-tauri/src/server/sdk.rs`. Don't edit them by hand: change the API (and its `OPS` table), then run

```bash
cd src-tauri && UPDATE_SDK=1 cargo test --lib sdk
```

A test fails whenever the committed clients differ from what the generator produces, and another drives both against a live server. A running server also serves them: `GET /api/v1/sdk/typescript` and `GET /api/v1/sdk/python`.

## TypeScript (`typescript/src/index.ts`)

Fetch-based, no dependencies (Node 18+, browsers, Deno, Bun).

```ts
import { UfopClient, UfopError } from "./src/index.ts";

const ufop = new UfopClient("https://backup.example.com:8200", { token: process.env.UFOP_TOKEN }); // an API token (ufop_pat_…)
const jobs = await ufop.getJobs();
const { task_id } = await ufop.postJobsIdRun(jobs[0].id as string);
try {
  await ufop.getJobsId("missing");
} catch (e) {
  if (e instanceof UfopError) console.log(e.status, e.message, e.advice);
}
```

## Python (`python/ufop_client.py`)

Standard library only (Python 3.8+).

```python
from ufop_client import UfopClient, UfopError

ufop = UfopClient("https://backup.example.com:8200", token=os.environ["UFOP_TOKEN"])
for job in ufop.get_jobs():
    print(job["name"])
ufop.post_jobs_id_run(job["id"], {"dry_run": True})
```

Method names follow the paths (`GET /api/v1/jobs/{id}/versions` → `getJobsIdVersions` / `get_jobs_id_versions`); each method's documentation names the API-token scope it needs. `login(password)` signs in with the server password instead of a token.
