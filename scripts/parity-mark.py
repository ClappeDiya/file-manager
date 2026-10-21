#!/usr/bin/env python3
"""Mark parity-tracker items: parity-mark.py <nums> <work> <verify> <evidence>
nums: comma list / ranges (e.g. 3,12,272-279). work: built|verified|in_progress|blocked.
verify: live|tests|device|account|docs."""
import json, sys
nums, work, verify, evidence = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
ids = set()
for part in nums.split(","):
    a, _, b = part.partition("-")
    ids.update(range(int(a), int(b or a) + 1))
p = "docs/duplicati-parity/features.json"
d = json.load(open(p))
hit = 0
for i in d["items"]:
    if i["n"] in ids:
        i.update(work=work, verify=verify, evidence=evidence, status="present"); hit += 1
json.dump(d, open(p, "w"), indent=1)
print(f"marked {hit} item(s)")
