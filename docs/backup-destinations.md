# Backup destinations

A backup job writes to one primary destination. It can also keep **replicas**: full copies on other destinations, synced after every backup, on an interval, or manually.

Every destination below is available for primaries, replicas and the destination library.

| Provider | Settings | Credentials | Verified with |
|---|---|---|---|
| Local / removable / mounted share | folder, alternate paths, marker file, atomic writes, size check, fsync | — | real filesystem |
| SMB (native, no mount) | server, share, folder, user, domain, minimum protocol (SMB2 / SMB3 / SMB3_11), encryption | password | Samba 4.19 with `server min protocol = SMB3_11` |
| Amazon S3 & S3-compatible | service preset (AWS, IDrive e2, Storj gateway, Aliyun OSS, Tencent COS, Wasabi, MinIO), endpoint, region, bucket, folder, storage class, multipart part size, path-style, archive-restore policy, Object Lock bucket creation, TLS trust | secret key (+ newline + session token) | moto: uploads incl. multipart, Object Lock (compliance blocks deletion), Glacier restore |
| Azure Blob | account, container, folder, endpoint, access tier, block size, rehydration priority | account key or SAS | Azurite: block-list uploads, full backup/restore |
| Google Cloud Storage | bucket, folder, project, storage class, endpoint, CRC32C/MD5 checksums | service-account JSON or access token | fake-gcs-server: multipart uploads with checksums, full backup/restore |
| Tahoe-LAFS | gateway URL, directory capability, folder | — | real Tahoe-LAFS 1.20 grid |
| rclone bridge (MEGA, Jottacloud, Koofr, …) | remote, folder, executable | rclone config password | rclone 1.60 (`:local:` remote) |
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
