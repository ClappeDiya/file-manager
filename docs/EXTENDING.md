# Extending the backup engine

The backup engine in `crates/ufop-core` (feature `backup`) has three extension points. Each is a Rust trait. You register an implementation in a `ModuleRegistry` (`ufop_core::backup::modules`).

| Kind | Trait | Built-ins | Used for |
|------|-------|-----------|----------|
| Encryption | `EncryptionModule` | AES-256-GCM (passphrase), none | Sealing and opening whole remote volumes |
| Compression | `CompressionModule` | `deflate`, `stored`, `bzip2`, `zstd` | Per-block compression |
| Destination | `Backend` | `file` (local, removable, UNC/mounted shares) | Where volumes are stored |

The desktop app and `ufop-server` also register every protocol connector as a destination, through `src-tauri/src/backup_engine/connector_backend.rs`.

## Building from source

```bash
cd crates/ufop-core && cargo test --features backup   # engine + module tests
cd src-tauri && cargo build --bin ufop-server          # headless server
cd cli && cargo build                                   # `ufop backup …`
```

## Encryption module

```rust
use std::sync::Arc;
use ufop_core::backup::crypto::VolumeCipher;
use ufop_core::backup::modules::EncryptionModule;
use ufop_core::error::AppError;

struct MyCipher { /* key material */ }

impl EncryptionModule for MyCipher {
    fn name(&self) -> &str { "mycipher" }
    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, AppError> { /* … */ todo!() }
    fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, AppError> { /* … */ todo!() }
}

// Use it for a backup:
let cipher = VolumeCipher::external(Arc::new(MyCipher { /* … */ }));
ufop_core::backup::engine::run_backup_with_cipher(
    &backend, &db, &sources, &filter, &options, &retention,
    None, Some(cipher), kdf, &ctx,
)?;
```

Volumes sealed by an external module carry a marker byte. A cipher without that module refuses to open them and shows a clear error, rather than misreading the data.

## Compression module

```rust
use ufop_core::backup::modules::{CompressionModule, ModuleRegistry};

struct Lz4;
impl CompressionModule for Lz4 {
    fn name(&self) -> &str { "lz4" }
    fn compress(&self, data: &[u8], level: u32) -> Result<Vec<u8>, AppError> { todo!() }
    fn decompress(&self, data: &[u8]) -> Result<Vec<u8>, AppError> { todo!() }
}

let mut registry = ModuleRegistry::builtin();
registry.register_compression(Arc::new(Lz4));
```

## Destination module

Implement `Backend` with these methods: `describe`, `list`, `put`, `get`, `delete` and `create_folder`.

Optional methods:

- `rename`: the default is copy + delete.
- `quota`: return it when the destination can report capacity. Quota warnings then work without a configured quota size.
- `test`: the connectivity check.

Volume names never contain path separators.

```rust
registry.register_backend("mystore", Arc::new(|url: &str| {
    Ok(Box::new(MyStore::connect(url)?) as Box<dyn Backend>)
}));
let backend = registry.backend("mystore://bucket/path")?;
```

Wrap a backend in `RetryingBackend` to get retries and throttling. Wrap it in `SoftDeleteBackend` to rename instead of delete.

## Hooks

`OpContext` provides these hooks:

- progress (`with_progress`);
- cancellation (`with_cancel_flag`, `cancel`);
- a callback that fires when the backup has finished reading its sources (`with_source_released`). The desktop app uses it for the *When sources are released* script.

## Testing your module

The engine's tests show the expected behaviour. `modules.rs` contains a round trip for every built-in compression module, plus an external XOR cipher that plugs into `VolumeCipher`. To exercise a destination, use `backend::test_backend_compatibility(&backend, files, min, max)`, the same code behind `ufop backup test-destination`.
