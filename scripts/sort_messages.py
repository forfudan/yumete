#!/usr/bin/env python3
"""Put each message in the file of its prefix, and each file in order.

The messages live in `crates/yumete-core/messages/`, one file per area; each
file's first line names the prefixes it holds (`# prefixes: cmd`). A test
checks both rules. **Run this after adding messages**: working out
alphabetical order by eye goes wrong (2026-09-22, the third time the same test
stopped a commit), and a new message is easiest added anywhere and moved here.
"""
import re
import sys
import pathlib

DIR = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "crates/yumete-core/messages")
BLOCK = re.compile(r"(?m)(?=^\[\[message\]\]$)")


def key_of(block):
    m = re.search(r'^key = "([^"]+)"', block, re.M)
    return m.group(1) if m else ""


files = {}
owner = {}
for path in sorted(DIR.glob("*.toml")):
    text = path.read_text()
    first = text.split("\n", 1)[0]
    if not first.startswith("# prefixes:"):
        sys.exit(f"{path.name}: the first line must name its prefixes")
    for prefix in first[len("# prefixes:"):].split():
        if prefix in owner:
            sys.exit(f"`{prefix}` is named by {owner[prefix]} and {path.name}")
        owner[prefix] = path.name
    start = BLOCK.search(text)
    head = text[: start.start()] if start else text.rstrip("\n") + "\n\n"
    blocks = [b for b in BLOCK.split(text[start.start():] if start else "") if b.strip()]
    files[path.name] = (head, blocks)

moved = []
gathered = {name: [] for name in files}
for name, (_, blocks) in files.items():
    for b in blocks:
        prefix = key_of(b).split(".")[0]
        home = owner.get(prefix)
        if home is None:
            sys.exit(f"{key_of(b)}: no file names the prefix `{prefix}`; add it to one file's first line")
        if home != name:
            moved.append(f"{key_of(b)}: {name} → {home}")
        gathered[home].append(b if b.endswith("\n\n") else b.rstrip("\n") + "\n\n")

changed = 0
for name, (head, _) in files.items():
    blocks = sorted(gathered[name], key=key_of)
    text = (head + "".join(blocks)).rstrip("\n") + "\n"
    path = DIR / name
    if path.read_text() != text:
        path.write_text(text)
        changed += 1
for line in moved:
    print(line)
print("in order" if changed == 0 else f"{changed} file(s) rewritten")
