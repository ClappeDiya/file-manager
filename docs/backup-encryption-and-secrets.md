# Backup encryption, certificate pinning and secret providers

## Encryption method

You choose the method when the backup is created (**Options → Encryption method**, or `ufop backup run --encryption-module`). The repository header (`<prefix>-repo.json`) records the method. Later runs, restores, database rebuilds and the recovery tool read it from the header.

| Method | Key | Volume names | Notes |
|---|---|---|---|
| AES-256-GCM (default) | Argon2id from the passphrase | `….aes` | The Argon2id cost is stored in the header. |
| ChaCha20-Poly1305 (format 2) | Argon2id from the passphrase | `….aes` | Pick it under **Options → Advanced → Repository format → Cipher** or with `--cipher chacha20-poly1305`. The header records the cipher. |
| GPG, symmetric | the passphrase (`gpg --symmetric --cipher-algo AES256`) | `….gpg` | Needs GnuPG. The passphrase goes to gpg on stdin, never on the command line. `--no-symkey-cache` stops gpg-agent from accepting a cached key. Volumes never touch disk unencrypted: the passphrase is the first line on stdin and the volume follows. |
| GPG, recipients | public keys (`--gpg-recipient`) | `….gpg` | Backups need no passphrase. Restores need the secret key in the keyring. |

AES and symmetric GPG backups carry a key check, so a wrong passphrase is rejected before any work starts.

### Envelope key (format 2)

In format 1 the Argon2id key encrypts every volume, so a new passphrase means re-encrypting the whole backup. Format 2 encrypts volumes with a random 256-bit master key. The header stores that key wrapped (AES-256-GCM or ChaCha20-Poly1305) by the Argon2id key, the key-encryption key. The key check proves the key-encryption key, so a wrong passphrase is still rejected at once.

**Change password** unwraps the master key with the old passphrase and wraps it with the new one. Only the header changes, written safely (temporary name, read back, previous header kept until success). The job's stored passphrase is updated too. Upgrading a format 1 backup keeps the old key as the master key, so the passphrase and all volumes keep working. See [backup-repository-format.md](backup-repository-format.md).

Each volume still gets its own random file key, wrapped with the volume key, as before. A volume's flag byte says which cipher sealed it (1 AES-256-GCM, 3 ChaCha20-Poly1305), so readers handle both.

### Key derivation

Argon2id cost is set per backup. It is checked to be at least 8 MiB of memory, 1–50 passes and 1–64 lanes.

| Preset | Memory | Passes | Lanes |
|---|---|---|---|
| `interactive` | 19 MiB | 2 | 1 |
| `default` | 64 MiB | 3 | 4 |
| `sensitive` | 256 MiB | 4 | 4 |

CLI: `--kdf-preset`, or `--kdf-memory-kib`, `--kdf-iterations` and `--kdf-parallelism`.

## Standalone encryption

**Recovery tool → Encrypt or decrypt files**, or the CLI:

```bash
ufop backup crypt encrypt report.pdf report.pdf.ufopenc --kdf-preset sensitive
ufop backup crypt decrypt report.pdf.ufopenc report.pdf
ufop backup crypt decrypt-volume ufop-b….dblock.aes --header ufop-repo.json volume.zip
```

`.ufopenc` files use the backup cipher. The data is split into 4 MiB authenticated chunks. Each chunk carries its index and a last-chunk flag, so reordering, dropped chunks and truncation are all detected. Output is written to a temporary file and renamed into place only when complete.

`decrypt-volume` turns one backup volume back into its plain ZIP or UFOP container.

## Certificate pinning

Each place that connects over TLS takes a list of SHA-256 certificate fingerprints: HTTP destinations, report URLs, email/XMPP/Telegram notifications and secret providers. When the list is set, only those certificates are accepted and the certificate chain is not consulted. This lets you pin a self-signed server, and it refuses everything else.

**Get fingerprint** (the `tls_fetch_fingerprint` command) reads the fingerprint a server presents, so you can compare it and pin it.

## Secret providers

Write a setting as `$name`, or put `${name}` inside a longer value, and the value is read from the configured providers when the job runs. `$$` writes a literal `$`. Filter rules are never treated as placeholders, so `$RECYCLE.BIN` stays as written.

Stored passwords can be placeholders too: the backup passphrase, destination secrets and notification passwords. The job keeps the placeholder. Resolved values are masked as `***` in run logs.

| Provider | Configuration | Credential (secure store) | Verified with |
|---|---|---|---|
| Environment | variable prefix | — | tests |
| File | JSON object or `.env` file | — | tests |
| `pass` | folder, executable | (GPG agent) | real `pass` + GnuPG 2.4 |
| HashiCorp Vault | address, KV mount, path, namespace, KV v1/v2; `path#key` | token | Vault 1.17 dev server |
| AWS Secrets Manager | region, access key id, endpoint; `id#json_key`, or one JSON secret | secret key (+ session token) | moto |
| Google Secret Manager | project, endpoint; `id#json_key` | service-account JSON or access token | API mock (incl. service-account JWT) |
| Azure Key Vault | vault URL, tenant, client id; `_` maps to `-` | client secret | API mock |

**Scope.** A provider is either global (used by every job) or opt-in; a job opts in under **Hooks → Secrets**. Lookup order is the job's own providers first, then the global ones.

**Cache.** Choose none, in memory with a lifetime, or persistent. Persistent also keeps values in the secure store, so runs keep working while a provider is briefly unreachable.

**Diagnostics.** **Test saved providers** reports, for each provider, whether its credential is stored, the latency, and which probe names were found. **Check placeholders** lists every placeholder in a job and which provider resolves it. Neither ever shows a value.

**CLI.** Use `--secret-provider env://PREFIX,file:///path,pass://folder` with a `$name` passphrase, or `--passphrase-file`.
