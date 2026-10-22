#!/usr/bin/env python3
"""Independent UFOP backup recovery: list and restore a backup folder
without UFOP itself.

Needs only Python 3.9+ and the `cryptography` package (for AES-256-GCM
and Argon2id; `pip install cryptography`). ZIP volumes using Zstandard
also need Python 3.14+ or the `zstandard` package.

    UFOP_PASSPHRASE=… ufop-recover.py list    /mnt/backup
    UFOP_PASSPHRASE=… ufop-recover.py restore /mnt/backup /tmp/out [--version 0] [--path PATTERN]…

The folder is the backup destination (local disk, removable drive or a
mounted share). Files are verified against their recorded hashes; files
with missing data are written with zero-filled gaps and reported.

Format (see crates/ufop-core/src/backup/{crypto,volume}.rs):
  <prefix>-repo.json        JSON header: encrypted, salt (hex), kdf
                            {memory_kib, iterations, parallelism}, key_check,
                            block_size, block_hash, file_hash, volume_format
  volumes                   "UFOPV1" + flag (0 plain, 1 AES, 2 external module)
                            flag 1: nonce(12) | AES-GCM(master, file key)(48)
                                    | nonce(12) | AES-GCM(file key, payload)
  dlist payload             JSON {timestamp, partial, files: [FileEntry]}
  dindex payload            JSON {block_volume, blocks: [{hash, size}]}
  dblock payload (ufop)     u32 count, then per block: u8 len | hash | u8 flag
                            (0 raw, 1 raw deflate) | u32 len | data
  dblock payload (zip)      ZIP archive, one entry per block named by its hash
"""
import argparse
import fnmatch
import hashlib
import io
import json
import os
import re
import struct
import sys
import zipfile
import zlib

MAGIC = b"UFOPV1"
KEY_CHECK = b"ufop-backup-key-check"
TS_RE = re.compile(r"-(\d{8}T\d{6}Z)\.dlist")


class Fail(Exception):
    pass


def _aes():
    try:
        from cryptography.hazmat.primitives.ciphers.aead import AESGCM
    except ImportError:
        raise Fail("This backup is encrypted: install the 'cryptography' package (pip install cryptography)")
    return AESGCM


def derive_key(passphrase, header):
    try:
        from cryptography.hazmat.primitives.kdf.argon2 import Argon2id
    except ImportError:
        raise Fail("cryptography >= 44 is needed for Argon2id (pip install -U cryptography)")
    kdf = header.get("kdf") or {"memory_kib": 65536, "iterations": 3, "parallelism": 4}
    key = Argon2id(
        salt=bytes.fromhex(header["salt"]),
        length=32,
        iterations=kdf["iterations"],
        lanes=kdf["parallelism"],
        memory_cost=kdf["memory_kib"],
    ).derive(passphrase.encode())
    raw = bytes.fromhex(header.get("key_check") or "")
    try:
        if _aes()(key).decrypt(raw[:12], raw[12:], None) != KEY_CHECK:
            raise Fail("Key check mismatch")
    except Fail:
        raise
    except Exception:
        raise Fail("The passphrase is incorrect")
    return key


def open_volume(data, key):
    if len(data) < 7 or data[:6] != MAGIC:
        raise Fail("not a UFOP volume")
    flag = data[6]
    if flag == 0:
        return data[7:]
    if flag == 1:
        if key is None:
            raise Fail("volume is encrypted but no passphrase was given")
        body = data[7:]
        if len(body) < 12 + 48 + 12 + 16:
            raise Fail("encrypted volume is truncated")
        aes = _aes()
        try:
            file_key = aes(key).decrypt(body[:12], body[12:60], None)
            return aes(file_key).decrypt(body[60:72], body[72:], None)
        except Exception:
            raise Fail("decryption failed (damaged volume)")
    if flag == 2:
        raise Fail("volume uses an external encryption module (e.g. GPG); decrypt it with that tool first")
    raise Fail("unknown volume flag %d" % flag)


def _zstd(raw):
    try:
        import compression.zstd as z  # Python 3.14+
        return z.decompress(raw)
    except ImportError:
        pass
    try:
        import zstandard
    except ImportError:
        raise Fail("ZIP volume uses Zstandard: use Python 3.14+ or pip install zstandard")
    return zstandard.ZstdDecompressor().decompressobj().decompress(raw)


def _zip_entry(buf, zf, info):
    try:
        return zf.read(info)
    except NotImplementedError:
        if info.compress_type != 93:
            raise
        # Read the raw entry data and inflate it with zstd ourselves.
        hdr = buf[info.header_offset:info.header_offset + 30]
        name_len, extra_len = struct.unpack("<HH", hdr[26:30])
        start = info.header_offset + 30 + name_len + extra_len
        return _zstd(buf[start:start + info.compress_size])


def read_blocks(payload):
    """hash -> bytes for one block volume payload."""
    out = {}
    if payload[:4] in (b"PK\x03\x04", b"PK\x05\x06"):
        zf = zipfile.ZipFile(io.BytesIO(payload))
        for info in zf.infolist():
            out[info.filename] = _zip_entry(payload, zf, info)
        return out
    p = 0

    def take(n):
        nonlocal p
        if p + n > len(payload):
            raise Fail("block volume is truncated")
        s = payload[p:p + n]
        p += n
        return s

    count = struct.unpack("<I", take(4))[0]
    if count > len(payload) // 6:
        raise Fail("block volume is corrupt")
    for _ in range(count):
        h = take(take(1)[0]).decode()
        flag = take(1)[0]
        d = take(struct.unpack("<I", take(4))[0])
        if flag == 0:
            out[h] = d
        elif flag == 1:
            out[h] = zlib.decompress(d, -15)
        else:
            raise Fail("unknown block flag %d" % flag)
    return out


def hash_hex(alg, data):
    return hashlib.new("sha512" if alg == "sha512" else "sha256", data).hexdigest()


class Backup:
    def __init__(self, folder, passphrase, prefix):
        self.folder = folder
        self.prefix = prefix
        hp = os.path.join(folder, prefix + "-repo.json")
        if not os.path.isfile(hp):
            raise Fail("no %s-repo.json in %s (wrong folder or --prefix?)" % (prefix, folder))
        with open(hp, "rb") as f:
            self.header = json.load(f)
        self.key = None
        if self.header.get("encrypted"):
            if not passphrase:
                raise Fail("this backup is encrypted: set UFOP_PASSPHRASE or use --passphrase")
            self.key = derive_key(passphrase, self.header)
        names = sorted(os.listdir(folder))
        self.names = [n for n in names if n.startswith(prefix + "-") and not n.endswith(".par2")]

    def kind(self, name):
        rest = name[len(self.prefix) + 1:]
        if rest.endswith(".aes"):
            rest = rest[:-4]
        return rest.rsplit(".", 1)[-1] if "." in rest else None

    def payload(self, name):
        with open(os.path.join(self.folder, name), "rb") as f:
            return open_volume(f.read(), self.key)

    def versions(self):
        """Newest first: (name, filelist)."""
        out = []
        for n in self.names:
            if self.kind(n) != "dlist":
                continue
            try:
                out.append((n, json.loads(self.payload(n))))
            except (Fail, ValueError) as e:
                print("warning: %s unreadable: %s" % (n, e), file=sys.stderr)
        out.sort(key=lambda v: v[1]["timestamp"], reverse=True)
        return out

    def block_index(self, needed):
        """hash -> volume name, from index volumes, then by scanning block volumes."""
        where = {}
        for n in self.names:
            if self.kind(n) == "dindex":
                try:
                    idx = json.loads(self.payload(n))
                    for b in idx["blocks"]:
                        where[b["hash"]] = idx["block_volume"]
                except (Fail, ValueError, KeyError):
                    pass
        present = set(self.names)
        if any(h not in where or where[h] not in present for h in needed):
            for n in self.names:
                if self.kind(n) == "dblock":
                    try:
                        for h in read_blocks(self.payload(n)):
                            where.setdefault(h, n)
                    except Fail as e:
                        print("warning: %s: %s" % (n, e), file=sys.stderr)
        return {h: v for h, v in where.items() if v in present}


def selected(path, patterns):
    if not patterns:
        return True
    p = path.replace("\\", "/")
    for s in patterns:
        s = s.replace("\\", "/")
        if any(c in s for c in "*?["):
            if fnmatch.fnmatchcase(p, s):
                return True
        else:
            t = s.rstrip("/")
            if p == t or p.startswith(t + "/"):
                return True
    return False


def relative_target(paths):
    """Shortest common parent, as UFOP's default restore layout."""
    parents = [os.path.dirname(p.replace("\\", "/")) for p in paths]
    common = os.path.commonpath(parents) if parents else ""
    return common


def cmd_list(b, args):
    for i, (n, fl) in enumerate(b.versions()):
        files = [f for f in fl["files"] if f["kind"] == "file"]
        size = sum(f["size"] for f in files)
        print("%d\t%s\t%d files\t%d bytes%s" % (i, fl["timestamp"], len(files), size, "\tpartial" if fl.get("partial") else ""))
        if args.files:
            for f in fl["files"]:
                print("\t%s\t%s\t%d" % (f["kind"], f["path"], f["size"]))
    return 0


def cmd_restore(b, args):
    vs = b.versions()
    if args.version >= len(vs):
        raise Fail("version %d not found (%d available)" % (args.version, len(vs)))
    name, fl = vs[args.version]
    entries = [e for e in fl["files"] if selected(e["path"], args.path)]
    if not entries:
        raise Fail("no files match")
    base = relative_target([e["path"] for e in entries])
    bs = b.header["block_size"]
    needed = {h for e in entries if e["kind"] == "file" for h in e["blocks"]}
    where = b.block_index(needed)

    def dest(path):
        p = path.replace("\\", "/")
        rel = os.path.relpath(p, base) if base else p.lstrip("/")
        rel = rel.replace(":", "_") if os.name == "nt" else rel
        return os.path.join(args.target, rel)

    jobs = {}
    for i, e in enumerate(entries):
        d = dest(e["path"])
        if e["kind"] == "dir":
            os.makedirs(d, exist_ok=True)
        elif e["kind"] == "symlink":
            os.makedirs(os.path.dirname(d), exist_ok=True)
            if e.get("symlink_target") and hasattr(os, "symlink"):
                try:
                    if os.path.lexists(d):
                        os.remove(d)
                    os.symlink(e["symlink_target"], d)
                except OSError as err:
                    print("warning: %s: %s" % (e["path"], err), file=sys.stderr)
        else:
            os.makedirs(os.path.dirname(d), exist_ok=True)
            with open(d, "wb") as f:
                f.truncate(e["size"])
            for bi, h in enumerate(e["blocks"]):
                jobs.setdefault(where.get(h), []).append((i, bi * bs, h))
    missing = {}
    for vol, items in jobs.items():
        blocks = {}
        if vol is not None:
            try:
                blocks = read_blocks(b.payload(vol))
            except Fail as err:
                print("warning: %s: %s" % (vol, err), file=sys.stderr)
        for i, off, h in items:
            data = blocks.get(h)
            if data is None or hash_hex(b.header["block_hash"], data) != h:
                missing[i] = missing.get(i, 0) + 1
                continue
            with open(dest(entries[i]["path"]), "r+b") as f:
                f.seek(off)
                f.write(data)
    ok = partial = 0
    for i, e in enumerate(entries):
        if e["kind"] != "file":
            continue
        d = dest(e["path"])
        if missing.get(i):
            partial += 1
            print("PARTIAL %s (%d block(s) missing)" % (e["path"], missing[i]), file=sys.stderr)
        elif e.get("hash"):
            with open(d, "rb") as f:
                if hash_hex(b.header["file_hash"], f.read()) != e["hash"]:
                    partial += 1
                    print("MISMATCH %s" % e["path"], file=sys.stderr)
                    continue
            ok += 1
        else:
            ok += 1
        if e.get("mtime") is not None:
            os.utime(d, (e["mtime"], e["mtime"]))
    print("Restored %d file(s) from %s; %d incomplete" % (ok, fl["timestamp"], partial))
    return 0 if partial == 0 else 1


def main(argv=None):
    ap = argparse.ArgumentParser(description="Recover files from a UFOP backup folder without UFOP.")
    ap.add_argument("--prefix", default="ufop")
    ap.add_argument("--passphrase", help="prefer the UFOP_PASSPHRASE environment variable")
    sub = ap.add_subparsers(dest="cmd", required=True)
    pl = sub.add_parser("list", help="list versions")
    pl.add_argument("folder")
    pl.add_argument("--files", action="store_true", help="also list every entry")
    pr = sub.add_parser("restore", help="restore files")
    pr.add_argument("folder")
    pr.add_argument("target")
    pr.add_argument("--version", type=int, default=0, help="0 = newest")
    pr.add_argument("--path", action="append", default=[], help="path, folder or wildcard (repeatable)")
    args = ap.parse_args(argv)
    try:
        b = Backup(args.folder, args.passphrase or os.environ.get("UFOP_PASSPHRASE"), args.prefix)
        return cmd_list(b, args) if args.cmd == "list" else cmd_restore(b, args)
    except Fail as e:
        print("error: %s" % e, file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
