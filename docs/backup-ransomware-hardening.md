# Hardening backups against ransomware

Ransomware that takes over a machine also takes over everything that machine can do: its files, its saved
passwords and the credentials its backup jobs use. A backup survives only if those credentials **cannot
destroy it**, and if a copy exists that the machine cannot reach at all. This guide shows how to set that up
with each kind of destination, and what the app checks for you.

Each backup job shows **Ransomware protection** in the job editor (*Destination* step): a badge such as
*Protection: 4/7* in Standard mode, and in Advanced mode the checklist with what to change and **Run checks**,
which writes a small probe object to the destination and tries to delete it with the job's credentials
(only when you click it; the probe is removed afterwards when the credentials allow it). The same report is
available as `ufop backup hardening <job> [--probe]`, `GET /api/v1/jobs/{id}/hardening`,
`POST /api/v1/jobs/{id}/hardening/probe` and the `backup_hardening_report` command.

| Check | Passes when |
|---|---|
| Backups are encrypted | The job encrypts its volumes ([Encrypt everything](#encrypt-everything)) |
| New volumes are locked | Object lock is on and the destination can lock ([Object lock and immutability](#object-lock-and-immutability)) |
| Deleted volumes are kept for a while | A soft-delete prefix is set ([Versioning, soft delete and lifecycle](#versioning-soft-delete-and-lifecycle)) |
| A copy is kept with another provider | An enabled replica uses another provider or an offline disk ([Offline and second-provider copies](#offline-and-second-provider-copies)) |
| The local database is encrypted | [Protect this machine](#protect-this-machine) |
| Enough versions are kept | The retention keeps about a week or more of versions; pins are listed ([Retention and pins](#retention-and-pins)) |
| The backup credentials cannot delete | The last *Run checks* could not delete the probe ([Least-privilege credentials](#least-privilege-credentials)) |
| Legal hold (information) | Shown when a hold is placed ([Legal hold](#legal-hold)) |

Settings that the provider holds (bucket versioning, retention policies, IAM) are invisible to the app
except through the probe. Treat the checklist as a starting point, and verify the provider side with this
guide.

## Encrypt everything

Encrypt every job (AES-256-GCM with an Argon2id-derived key; *General → Encryption*). Encryption cannot be
added to an existing repository: create a new encrypted job and let the old one age out. Keep the
passphrase **offline** as well (a password manager not on the backed-up machine, or on paper in a safe):
an attacker who deletes the machine's credential store must not take the only copy with it. Encryption
also makes tampering detectable: a modified volume fails its authentication tag on verification.

## Least-privilege credentials

The credentials stored for a backup job should be able to **write new objects and read and list existing
ones, but not delete or overwrite them**. Then ransomware on the machine can add garbage, but cannot remove
what is already there. What that means per provider:

### Amazon S3 and S3-compatible storage

An IAM policy for the backup identity (replace `my-backups` and the prefix):

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Sid": "ListTheBackupPrefix",
      "Effect": "Allow",
      "Action": ["s3:ListBucket", "s3:GetBucketLocation", "s3:GetBucketObjectLockConfiguration", "s3:GetBucketVersioning"],
      "Resource": "arn:aws:s3:::my-backups",
      "Condition": { "StringLike": { "s3:prefix": ["office-pc/*", "office-pc"] } }
    },
    {
      "Sid": "WriteAndReadVolumes",
      "Effect": "Allow",
      "Action": ["s3:PutObject", "s3:GetObject", "s3:PutObjectRetention", "s3:GetObjectRetention"],
      "Resource": "arn:aws:s3:::my-backups/office-pc/*"
    },
    {
      "Sid": "NeverDelete",
      "Effect": "Deny",
      "Action": ["s3:DeleteObject", "s3:DeleteObjectVersion", "s3:BypassGovernanceRetention", "s3:PutBucketPolicy", "s3:PutLifecycleConfiguration", "s3:PutBucketVersioning", "s3:PutBucketObjectLockConfiguration"],
      "Resource": ["arn:aws:s3:::my-backups", "arn:aws:s3:::my-backups/*"]
    }
  ]
}
```

`s3:PutObject` alone still lets a client **overwrite** an object of the same name. With bucket versioning
(next section) an overwrite only adds a new version, so the original stays recoverable. A bucket policy can
add `"Condition": {"Bool": {"aws:SecureTransport": "false"}}` denies and restrict the backup identity to the
office's IP ranges (`aws:SourceIp`). With STS *AssumeRole* (destination → credential source), give the role
this policy and keep the base credentials only able to assume it.

MinIO, Wasabi, Backblaze's S3 API, Cloudflare R2 and other S3-compatible services accept the same policy
language (MinIO: `mc admin policy create`); check which actions each supports.

### Backblaze B2 (native API)

Create an **application key** restricted to the bucket (and a name prefix) with the capabilities
`listBuckets, listFiles, readFiles, writeFiles` — **without** `deleteFiles`, `writeBucketRetentions` or
`bypassGovernance`:

```bash
b2 key create --bucket my-backups --name-prefix office-pc/ office-pc-backup listBuckets,listFiles,readFiles,writeFiles
```

B2 keeps every upload as a file version; "hiding" a file does not delete it. Pair it with a lifecycle rule
that deletes hidden versions after N days (set it with the maintenance key or in the web console).

### Azure Blob Storage

Use a **SAS** (or a user-delegation SAS with Entra ID) with only `r` (read), `a` (add), `c` (create),
`w` (write) and `l` (list) on the container — no `d` (delete), `x` (delete version) or `y` (permanent
delete):

```bash
az storage container generate-sas --account-name acct --name backups --permissions racwl --expiry 2027-01-01 --https-only --auth-mode login --as-user
```

With Entra sign-in (service principal, managed identity), assign a custom role with
`Microsoft.Storage/storageAccounts/blobServices/containers/blobs/read`, `…/write` and `…/add/action` on the
container, rather than *Storage Blob Data Contributor* (which can delete).

### Google Cloud Storage

Grant the backup service account **Storage Object Creator** (`roles/storage.objectCreator`, create only)
plus **Storage Object Viewer** (`roles/storage.objectViewer`, read and list) on the bucket — not *Storage
Object Admin* or *Storage Admin*. Object Creator cannot delete or overwrite objects unless the request also
has delete permission, so an overwrite attempt fails.

### SFTP and other servers

See [Append-only SFTP](#append-only-sftp). For WebDAV and FTP servers, use an account whose server-side ACL
allows creating files but not deleting or renaming them, where the server supports it.

## The two-credential model

Backups **add** data; **compaction and retention remove** data (old versions, rewritten volumes, the
soft-delete folder). With put-only backup credentials those maintenance runs fail with "access denied" —
by design. Run them with a second identity that can delete, kept off the backed-up machine:

1. **Backup identity** (on the machine): put/get/list only, as above. Turn off automatic compaction and the
   retention part of maintenance for the job (*Options → Maintenance*), or leave it on and accept that it
   will report "access denied".
2. **Maintenance identity** (elsewhere): delete rights on the same prefix, used from a separate place — a
   `ufop-server` or a second job on another, better protected machine pointed at the same repository
   (*Add existing backup*), which runs compaction, retention and *Delete versions* on a schedule. Store its
   credentials only there.
3. With object lock, the maintenance identity still cannot delete locked volumes before their retention
   ends; set the lock period a little longer than the time it would take you to notice an attack (two to
   four weeks is common).

The repository lock (`ufop-lock.json`) is written and removed by both identities; the backup identity needs
to delete its own lock file. Grant `s3:DeleteObject` (or the provider's equivalent) on exactly
`<prefix>/ufop-lock*` to the backup identity if you see "could not release the repository lock", or rely
on lock expiry (*Options → Repository safety → lock lease*).

## Object lock and immutability

Object lock makes each object undeletable and unchangeable until a date, enforced by the provider:

| Provider | Feature | Notes |
|---|---|---|
| Amazon S3 (and S3-compatible with lock support) | **Object Lock** on a versioned bucket (enable at bucket creation) | *Governance* mode: users with `s3:BypassGovernanceRetention` can lift it. *Compliance* mode: nobody, not even the root account, until the date |
| Backblaze B2 | **Object Lock** on the bucket | Governance and compliance like S3 |
| Azure Blob | **Version-level immutability** (WORM) policies on the container or account | Time-based retention; *locked* policies cannot be shortened |
| Google Cloud Storage | **Bucket retention policy** or **object retention** | A *locked* bucket retention policy cannot be removed or shortened |

Turn on *Options → Repository safety → Object lock* in the job (days and mode): every new volume is locked
for that many days after upload, and *Maintenance → Apply locks* extends locks on existing volumes. Choose
**compliance** mode once you are sure of the period; governance is useful while testing. Compaction cannot
remove a locked volume early; the app reports it and tries again after the lock ends.

Default retention configured on the bucket (S3 *default retention*, GCS *bucket retention*) protects
objects even when the uploading client does not ask for a lock — set it as a safety net.

## Append-only SFTP

An SFTP server can refuse deletes and overwrites for the backup account:

- **rrsync / restricted shell**: OpenSSH's `internal-sftp` with a `ForceCommand` and a `ChrootDirectory`
  limits the account to its folder:

  ```text
  Match User office-pc
      ChrootDirectory /srv/backups/office-pc
      ForceCommand internal-sftp -P remove,rmdir,rename,posix-rename,setstat,fsetstat
      AllowTcpForwarding no
      X11Forwarding no
  ```

  `-P` (deny list, OpenSSH 6.5+) refuses deletes, renames and attribute changes. The chroot must be owned by
  root; give the account a writable subfolder and point the job at it.
- **Filesystem attributes**: on Linux, `chattr +a` (append-only) on the folder prevents removing entries,
  and a periodic `chattr +i` (immutable) on finished volumes, run by root, freezes them.
- **Snapshots**: on a ZFS or Btrfs server, take read-only snapshots of the backup dataset every few hours
  (`zfs snapshot`, `zfs-auto-snapshot`, `btrbk`); a client cannot touch them.

As with object stores, run compaction and retention from a maintenance account or on the server itself.

## Versioning, soft delete and lifecycle

Versioning keeps the previous content of every overwritten or deleted object:

- **S3**: enable bucket versioning; add a lifecycle rule that expires *noncurrent versions* after N days
  (`NoncurrentVersionExpiration`) and removes expired delete markers. Do not let the backup identity change
  the lifecycle (`s3:PutLifecycleConfiguration` is denied in the policy above).
- **Azure**: enable blob versioning and **soft delete** for blobs and containers (retention 7–365 days).
- **GCS**: enable object versioning or **soft delete** (default 7 days, up to 90) with a lifecycle rule
  for noncurrent objects.
- **B2**: file versions are always kept; set the bucket's lifecycle "keep prior versions for N days".

The job's own **soft-delete prefix** (*Options → Repository safety*) makes the app rename volumes it would
delete (compaction, retention) with a prefix instead of deleting them, for destinations without versioning.
It protects against mistakes and buggy maintenance, not against an attacker who uses the credentials
directly — combine it with put-only credentials.

## Offline and second-provider copies

Keep at least one copy that the backed-up machine cannot reach or cannot delete:

- **Replicas** (*Destination → Replicas*) mirror the repository to another destination after each backup
  or on their own schedule. Use **another provider or account** (for example S3 primary, B2 replica) with
  its own put-only credentials and object lock. Turn off *delete extraneous files* on a replica you want to
  keep longer than the primary, or give it its own lock.
- An **offline disk**: a replica to a USB disk that is connected only for the copy (a local destination with
  a marker file and a schedule), then unplugged and stored elsewhere. Rotate two disks.
- An **air-gapped** copy: run the replica from a machine without network access to the primary site, or
  pull the repository from the bucket with a separate machine (`ufop backup mirror`, `rclone sync` with
  read-only source credentials).
- A **remote server pull**: a `ufop-server` at another site that runs its own job against the same
  repository (read-only) or copies versions (*Copy versions*) into its own repository.

Test restores from the second copy regularly (*Verify*, *Restore → dry run*).

## Protect this machine

- **Encrypt the local database** (*Options → Repository safety*): it holds every backed-up file's name,
  size and hash.
- Keep the job passphrase and destination secrets in the OS credential store (the default), not in scripts
  or environment variables, and use secret providers (*Settings → Secret providers*) for servers.
- Run `ufop-server` as its own unprivileged user with an owner-only data directory (it enforces `0700`),
  behind HTTPS, with accounts and per-job access rules (see `server-operations.md`) so that a compromised
  operator account cannot delete jobs.
- Watch for anomalies: notifications on warnings and errors, Prometheus alerts on
  `ufop_backup_last_success_timestamp_seconds` and on sudden jumps in `ufop_backup_last_uploaded_bytes`
  (encrypted-by-ransomware files all change at once).

## Retention and pins

Ransomware often runs unnoticed for days before the ransom note. Keep enough history that clean versions
are still there when you find out: at least two weeks of daily versions plus weekly and monthly ones (for
example *daily 14, weekly 8, monthly 12*). **Pin** known-good versions (*Versions → Pin*) before risky
changes or after an incident: retention never removes pinned versions. Object lock should cover at least the
time to notice an attack; retention decides how long beyond that you keep data.

## Legal hold

A **legal hold** (*Maintenance → Legal hold*) suspends retention and refuses every removal (versions,
files, the job) until it is released; both actions are recorded in the audit log. Place one as soon as an
incident is suspected, so that no scheduled maintenance removes evidence or the last clean versions while you
investigate. Release it when the incident is closed.
