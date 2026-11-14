# Backup destinations

A backup job writes to one primary destination. It can also keep **replicas**: full copies on other destinations, synced after every backup, on an interval, or manually.

Every destination below is available for primaries, replicas and the destination library.

| Provider | Settings | Credentials | Verified with |
|---|---|---|---|
| Local / removable / mounted share | folder, alternate paths, marker file, atomic writes, size check, fsync | — | real filesystem |
| SMB (native, no mount) | server, share, folder, user, domain, minimum protocol (SMB2 / SMB3 / SMB3_11), encryption | password | Samba 4.19 with `server min protocol = SMB3_11` |
| Amazon S3 & S3-compatible | service preset (AWS, IDrive e2, Storj gateway, Aliyun OSS, Tencent COS, Wasabi, MinIO), endpoint, region, bucket, folder, storage class (also per volume kind), multipart part size, path-style, archive-restore policy, Object Lock bucket creation, TLS trust, credential source, assume-role, point in time | secret key (+ newline + session token) for static keys; nothing for the other credential sources | moto: uploads incl. multipart, Object Lock (compliance blocks deletion), Glacier restore; signature-checking S3/STS/IMDS/ECS mock: credential chain, assume-role refresh, classes per kind, point-in-time reads, sharded layouts |
| Azure Blob | account, container, folder, endpoint, access tier (also per volume kind), block size, rehydration priority, sign-in method, Entra authority | account key or SAS; client secret or PFX password for service principals | Azurite: block-list uploads, full backup/restore; Blob + Entra ID mock: every sign-in method, sharded layouts |
| Google Cloud Storage | bucket, folder, project, storage class (also per volume kind), endpoint, CRC32C/MD5 checksums, OAuth scope | service-account JSON or access token | fake-gcs-server: multipart uploads with checksums, full backup/restore; JSON API mock: JWT signature and scope checks, sharded layouts |
| Tahoe-LAFS | gateway URL, directory capability, folder | — | real Tahoe-LAFS 1.20 grid |
| rclone bridge (MEGA, Jottacloud, Koofr, …) | remote, folder, executable, call and startup timeouts, environment | rclone config password | rclone 1.60 (`:local:` remote); a fake rclone for timeouts, process-group kills, environment and stderr |
| Backblaze B2 | key id, bucket, folder | application key | API-level tests (in-process mock) |
| OpenStack Swift | Keystone v3 / v1 auth, project, region, container | password | API-level tests |
| Dropbox / Google Drive / Box / pCloud | folder (pCloud: US/EU region) | OAuth token JSON (refreshed automatically) | API-level tests incl. token refresh |
| OneDrive / SharePoint / Microsoft 365 group | `me`, `site:<id>`, `group:<id>` or `drive:<id>`, folder | OAuth token JSON | API-level tests incl. 5 MiB upload-session chunks |
| Filen (end-to-end encrypted) | account e-mail, folder, move deletions to trash | password, or JSON `{"password", "api_key", "two_factor_code"}` | API-level tests (gateway/ingest/egest mock that decrypts what it receives) + crypto test vectors from the SDK's code path |
| Drime Cloud | folder, workspace id, page size, move deletions to trash | API token, or JSON `{"email", "password"}` | API-level tests incl. S3 multipart (5 MiB parts) and listings deeper than 20 pages |
| Movistar Cloud (MiCloud) | account e-mail, browser device id, folder, listing limit, upload-validation wait | password | API-level tests incl. session expiry and upload validation |

The provider picker labels each entry with its verification level. Providers only tested against API mocks are marked *untested* until someone runs them against the live service. Box, pCloud and Movistar Cloud are hidden unless **Show untested providers** is on.

### Filen

Volumes are encrypted on this machine in Filen's own format, so they open in Filen's web, desktop and mobile apps. The format follows the official SDKs ([filen-sdk-ts](https://github.com/FilenCloudDienste/filen-sdk-ts), [filen-rs](https://github.com/FilenCloudDienste/filen-rs)):

- **Accounts with auth version 2** (PBKDF2-SHA512, 200 000 rounds; what the SDK registers new accounts with). Version 1 is deprecated. Version 3 (Argon2id with a data-encryption key) is refused with a clear error, because its file-name hash uses an HMAC key that the two official SDKs derive differently; use the rclone bridge (`filen` backend) for those accounts.
- **Metadata:** written as version `002`, the SDK's default; `002` and `003` are read.
- **File data:** written as version 2 (32-character file key, AES-256-GCM per 1 MiB chunk, `iv ‖ ciphertext ‖ tag`); versions 2 and 3 are read.
- **Two-factor accounts:** store the credentials as JSON with an `api_key` (from `filen-cli`). The password is still required, because the master key is derived from it.

### Drime Cloud and Movistar Cloud

Neither service publishes an API reference. Both stores follow Duplicati's backends ([DrimeCloud](https://github.com/duplicati/duplicati/tree/master/Duplicati/Library/Backend/DrimeCloud), [MovistarCloud](https://github.com/duplicati/duplicati/tree/master/Duplicati/Library/Backend/MovistarCloud)), which drive the same web APIs. Drime's quota comes from `user/space-usage`, as in [PyDrime](https://pypi.org/project/pydrime/).

- **Drime** doesn't overwrite files: a re-uploaded volume replaces the old one by deleting it first. Folder listings are checked against the folder's item count, and an inconsistent listing is refused rather than used partially.
- **Movistar Cloud's** listing API has no paging, so a listing that reaches the configured limit is refused. Deleted volumes go to the Movistar trash. Movistar closes the cloud storage of customers who leave Movistar.

Credentials never go into the job configuration. Each cloud destination has a `credential_id`, and its secret is kept in the OS keychain or the encrypted settings store (`backup_set_destination_secret`).

## S3 credentials and roles

**Credentials from** (`credentials` in the S3 settings, `{"type": …}`) picks where the keys come from. Existing destinations keep `static`.

| Source | Where the keys come from |
|---|---|
| `static` (default) | The access key id in the settings and the secret (optionally `secret⏎session-token`) in the credential store |
| `environment` | `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN` (and `AWS_CREDENTIAL_EXPIRATION`) of the app or service |
| `profile` `{name, file}` | A profile of `~/.aws/credentials` and `~/.aws/config` (`AWS_SHARED_CREDENTIALS_FILE`, `AWS_CONFIG_FILE`; `file` replaces the credentials file; an empty name is `AWS_PROFILE` or `default`) |
| `web_identity` `{role_arn, token_file}` | `AssumeRoleWithWebIdentity` with an OIDC token file (EKS IRSA, GitHub Actions, …); the file is read again on every refresh |
| `instance_metadata` | The EC2 instance profile through IMDSv2 (session token first); off when `AWS_EC2_METADATA_DISABLED=true` |
| `auto` | The SDK order: environment → web identity (`AWS_ROLE_ARN` + `AWS_WEB_IDENTITY_TOKEN_FILE`) → profile (`AWS_PROFILE` or `default`, when it exists) → container credentials (`AWS_CONTAINER_CREDENTIALS_RELATIVE_URI` or `_FULL_URI`, with `AWS_CONTAINER_AUTHORIZATION_TOKEN[_FILE]`) → instance metadata |

Profiles are resolved like the AWS CLI for these keys: static keys (`aws_access_key_id`, `aws_secret_access_key`, `aws_session_token`), `role_arn` with `source_profile` (chained, up to four levels) or `credential_source = Environment | Ec2InstanceMetadata | EcsContainer`, and `role_arn` with `web_identity_token_file`. **Not supported:** `credential_process` (UFOP does not run external commands for credentials) and IAM Identity Center (`sso_*`, `sso_session`), whose cached tokens cannot be refreshed unattended. Such profiles fail with advice: export the session keys (`aws configure export-credentials --format env`) and use `environment`, or use a role.

**Assume an IAM role** (`assume_role`, Advanced) calls STS `AssumeRole` with the base credentials: `role_arn`, `session_name` (default `ufop`), `external_id` (for cross-account trust policies), `duration_secs` (900–43 200, default 3600), and optionally `sts_endpoint` (GovCloud, VPC endpoints) and `region` (default: the bucket's). The same signed request as the S3 connection's assume-role is used.

Temporary credentials (STS, web identity, container, instance metadata) are cached and fetched again five minutes before they expire, so long backups keep running. Endpoints follow the AWS SDK variables: `AWS_ENDPOINT_URL_STS`, `AWS_EC2_METADATA_SERVICE_ENDPOINT`, `AWS_CONTAINER_CREDENTIALS_FULL_URI` (and `UFOP_AWS_ECS_ENDPOINT` for the base of a relative container URI).

## Azure sign-in with Microsoft Entra ID

**Sign in with** (`auth`) chooses the method. The secret in the credential store holds what the method needs.

| Method | Settings | Secret |
|---|---|---|
| `shared_key` (default) | — | Account key; a SAS token is recognised too, as before |
| `sas` | — | SAS token |
| `service_principal_secret` | tenant id, client id | Client secret |
| `service_principal_certificate` | tenant id, client id, certificate file: a PEM with the private key and the certificate, or a PFX/P12 | PFX password (if any) |
| `federated_token` | tenant id, client id, token file (AKS workload identity: `AZURE_FEDERATED_TOKEN_FILE`) | — |
| `managed_identity` | client id of a user-assigned identity (empty = system-assigned) | — |

Entra methods get an OAuth token for `https://storage.azure.com/.default` (client-credentials grant; a certificate signs an RS256 client assertion with its SHA-1 thumbprint as `x5t`) and send it as a bearer token with `x-ms-version` 2021-08-06 (bearer tokens need 2017-11-09 or later). Tokens are cached and fetched again five minutes before they expire. Managed identities use `IDENTITY_ENDPOINT` + `IDENTITY_HEADER` (App Service, Functions, Container Apps) when set, otherwise the instance metadata service (`AZURE_POD_IDENTITY_AUTHORITY_HOST` overrides its host). **Entra authority** (Advanced) and `AZURE_AUTHORITY_HOST` select a sovereign cloud such as `https://login.microsoftonline.us`. The identity needs *Storage Blob Data Contributor* on the container (*Reader* is enough for read-only jobs). Share links need an account key or SAS. The Azure Key Vault secret provider signs in through the same code.

## GCS scopes

A service-account key asks Google for one OAuth scope. With **OAuth scope** (`scope`) on *Automatic* (empty), read-only jobs and point-in-time views ask for `devstorage.read_only` and everything else for `devstorage.full_control` (needed for object retention locks). An explicit scope, such as `devstorage.read_write`, always wins. Access tokens given directly as the secret carry whatever scope they were issued with.

## Storage class per volume kind

`storage_class_by_kind` (S3, GCS) and `access_tier_by_kind` (Azure), under Advanced, set a class for each kind of file: `dblock` (data blocks, read on restore), `dindex` (indexes), `dlist` (file lists; both read on every rebuild and verification) and `other` (header, verification file, locks). An empty entry uses the single storage class or access tier.

Classes that need a restore before every read are **refused** for every kind, as Kopia does: S3 `GLACIER` and `DEEP_ARCHIVE`, Azure `Archive`. Verification, compaction and restores must read volumes directly. Instant-access tiers such as S3 `GLACIER_IR`, `STANDARD_IA` or Azure `Cool`/`Cold` work. GCS has no thaw step (`ARCHIVE` is read directly at a higher price), so every GCS class is allowed. The single `storage_class` keeps its existing archive support with **cold-storage restore**.

## Point in time (versioned S3 buckets)

`point_in_time` makes an S3 destination read the bucket as it was at that time. Bucket versioning must be on. The store lists object versions (`?versions`) and, for each key, uses the newest version written at or before that time. A key whose newest entry then is a delete marker, or that did not exist yet, is absent. Reads use `versionId`. The view is always read-only: every write is refused before it reaches the bucket, and the job needs **Read-only**.

To recover after ransomware or an accidental purge, open the job's **Restore** view in Advanced mode and use **Open backup as it was at …**. This creates a read-only job named "*job* (as of *time*)", rebuilds its index from the old versions, and lets you restore as usual. The same is available as `backup_open_point_in_time`, `POST /api/v1/jobs/{id}/point-in-time` (`{"at": "<RFC 3339>", "name": …}`) and `ufop backup open-at <job> --point-in-time <time>` (signed-in server).

## rclone process control

Every rclone call runs in its own process group (Unix). The group is killed, including anything rclone started, when the call ends, when it exceeds **Call timeout** (`timeout_secs`), when it prints nothing for **Startup timeout** (`startup_timeout_secs`, e.g. while waiting for a password prompt), or when the operation is cancelled. On Windows the rclone process itself is killed. `env` adds variables (such as `RCLONE_*` options or proxies). `isolate_env` starts rclone with only `PATH`, `HOME`, the temp and config folders and `env_passthrough`. A failed call's stderr (last lines) goes into the error's advice. `--config` (`config_file`) and the config password (`RCLONE_CONFIG_PASS`, from the credential store) work as before.

## Request limits and listing parallelism

**Request limits** (job editor → Options → Network, Advanced; `request_limits` in the job) cap requests per second separately for listings, downloads, uploads and deletes, and the number of requests in flight (`max_concurrent_requests`). Each retry counts as a request. The limiter is a token bucket per kind, holding at most one second's worth (at least one request), and applies to every destination type. `list_parallelism` lists that many shard folders at once for sharded layouts: local and mounted folders, object-store prefixes (S3, Azure, GCS, B2, Swift) and connection folders. The CLI takes `--list-rate`, `--get-rate`, `--put-rate`, `--delete-rate`, `--max-concurrent-requests` and `--list-parallelism`.

## Sharded layouts on cloud destinations

The sharded layout (see [backup-repository-format.md](backup-repository-format.md)) works on every destination with folders or prefix keys: local and mounted folders, S3, Azure, GCS, B2 and Swift (prefix "folders", listed with a delimiter), and SFTP, FTP/FTPS, WebDAV and S3 **connections** (shard folders are created on first upload). Reshard moves existing repositories there. The other native stores (Dropbox, Google Drive, Box, pCloud, OneDrive, Tahoe-LAFS, rclone, SMB, Filen, Drime, Movistar Cloud) stay flat.

## Immutability

With **Lock new volumes** set, every new volume is locked through the destination's object-lock API:

- S3 Object Lock, in governance or compliance mode;
- B2 file retention;
- Azure version-level immutability;
- GCS object retention.

Retention and compaction skip locked volumes. Maintenance → **Object locks** can lock existing volumes or extend every lock.

## Legal hold and eDiscovery

Maintenance → **Legal hold** places a hold on a job with a matter (case reference) and a reason. While it is held:

- retention is suspended, so every backup keeps all versions;
- deleting versions and purging files, broken ones included, is refused, except as a dry run;
- the job itself cannot be deleted, and saving the job in the editor keeps the hold.

Placing and releasing a hold are written to the tamper-evident audit log (`security.legal_hold.place` / `security.legal_hold.release`). Retention applies again from the first backup after the hold is released.

**eDiscovery export** writes one zip (`ediscovery_export`, format `ufop-ediscovery/1`):

| Entry | Contents |
|---|---|
| `audit/audit-log.jsonl`, `audit/verification.json` | The complete audit log, and its hash-chain check at export time |
| `ledger/events.jsonl` | File operations in the time range under the paths of interest |
| `backups/<job>/` | The job's configuration without secrets, its legal hold, versions, and per version the backed-up files under the paths |
| `files/<job>/…` | Optional: those files restored from the newest version (2 GB limit by default) |
| `manifest.json` | The request, the result, and the SHA-256 and size of every other entry |

The zip is written next to the target as `.zip.part` and renamed only when complete. The export is recorded in the audit log together with the zip's SHA-256.

## Running the destination tests

The emulator tests run when the matching environment variable is set:

```bash
moto_server -p 5005 &                                                     # pip install "moto[server]"
azurite-blob --blobPort 10000 --skipApiVersionCheck --loose &             # npm i azurite
fake-gcs-server -scheme http -port 4443 -backend memory &                 # github.com/fsouza/fake-gcs-server
UFOP_TEST_S3=http://127.0.0.1:5005 UFOP_TEST_AZURE=http://127.0.0.1:10000 \
UFOP_TEST_GCS=http://127.0.0.1:4443 UFOP_TEST_SMB='127.0.0.1:445/share/user/pass' \
UFOP_TEST_TAHOE='http://127.0.0.1:3456|URI:DIR2:…' \
  cargo test --lib backup_engine::stores
```

The API-mock and rclone tests always run.

## Live provider tests

`backup_engine::stores::live_providers` runs a contract against **real** services: the backend compatibility test (random uploads, listing, downloads with hash check, deletes), then two backups, rebuilding the index from the destination and listing the versions, a byte-for-byte restore, full verification, deleting the older version, compaction and a second verification. Each provider runs only when its variables are set. Unset providers are printed as skipped, with the variables to set. Each run works under `ufop-live/<time>-<id>` and removes everything it wrote, even after a failure.

```bash
./scripts/live-providers.sh                          # every configured provider
UFOP_LIVE_USE_MOCKS=1 ./scripts/live-providers.sh    # plus the in-process S3, Azure and GCS mocks
```

The same cycle always runs against the mocks (`contract_cycle_against_mocks`), so the harness stays correct without accounts.

| Provider | Variables (secrets) |
|---|---|
| S3 / compatible | `UFOP_LIVE_S3_BUCKET`, `UFOP_LIVE_S3_KEY`, `UFOP_LIVE_S3_SECRET`; optional `UFOP_LIVE_S3_ENDPOINT`, `UFOP_LIVE_S3_REGION`, `UFOP_LIVE_S3_PATH_STYLE` |
| Azure Blob | `UFOP_LIVE_AZURE_ACCOUNT`, `UFOP_LIVE_AZURE_CONTAINER`, and `UFOP_LIVE_AZURE_KEY` or `UFOP_LIVE_AZURE_TENANT` + `UFOP_LIVE_AZURE_CLIENT_ID` + `UFOP_LIVE_AZURE_CLIENT_SECRET`; optional `UFOP_LIVE_AZURE_ENDPOINT` |
| Google Cloud Storage | `UFOP_LIVE_GCS_BUCKET`, `UFOP_LIVE_GCS_KEY_JSON` (service-account key JSON) |
| Backblaze B2 | `UFOP_LIVE_B2_KEY_ID`, `UFOP_LIVE_B2_KEY`, `UFOP_LIVE_B2_BUCKET` |
| OpenStack Swift | `UFOP_LIVE_SWIFT_AUTH_URL`, `UFOP_LIVE_SWIFT_USER`, `UFOP_LIVE_SWIFT_PASSWORD`, `UFOP_LIVE_SWIFT_PROJECT`, `UFOP_LIVE_SWIFT_CONTAINER`; optional `UFOP_LIVE_SWIFT_REGION` |
| Dropbox | `UFOP_LIVE_DROPBOX_TOKEN` (OAuth token JSON with a refresh token) |
| OneDrive | `UFOP_LIVE_ONEDRIVE_TOKEN`; optional `UFOP_LIVE_ONEDRIVE_DRIVE` (`me`, `site:<id>`, …) |
| Google Drive | `UFOP_LIVE_GDRIVE_TOKEN` |
| Filen | `UFOP_LIVE_FILEN_EMAIL`, `UFOP_LIVE_FILEN_PASSWORD` |
| Drime | `UFOP_LIVE_DRIME_TOKEN` |

`.github/workflows/live-providers.yml.disabled` runs the same script nightly and on demand, with these names as repository secrets. It is standalone, not part of `ci.yml`. Like every workflow in this repository it ships disabled (see `.github/workflows/README.md` and `scripts/check-github-actions.sh`). Rename it to `live-providers.yml` to turn it on.
