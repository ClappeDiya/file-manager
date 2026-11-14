#!/usr/bin/env python3
"""Build the parity-tracker document (docs/duplicati-parity/features.json ->
tracker doc JSON) for upload to the tracker artifact's database."""
import json, sys, datetime
src = json.load(open("docs/duplicati-parity/features.json"))
out = {"updated": datetime.datetime.now(datetime.timezone.utc).isoformat(),
       "items": [{k: i[k] for k in ("n", "name", "phase", "work", "verify", "note", "evidence")} for i in src["items"]]}
path = sys.argv[1] if len(sys.argv) > 1 else "/dev/stdout"
json.dump(out, open(path, "w"))
