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

The provider picker labels each entry with its verification level. Providers only tested against API mocks are marked *untested* until someone runs them against the live service. Box and pCloud are hidden unless **Show untested providers** is on.

Credentials never go into the job configuration. Each cloud destination has a `credential_id`, and its secret is kept in the OS keychain or the encrypted settings store (`backup_set_destination_secret`).

## Immutability

With **Lock new volumes** set, every new volume is locked through the destination's object-lock API:

- S3 Object Lock, in governance or compliance mode;
- B2 file retention;
- Azure version-level immutability;
- GCS object retention.

Retention and compaction skip locked volumes. Maintenance → **Object locks** can lock existing volumes or extend every lock.

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
