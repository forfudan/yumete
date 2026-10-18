#!/usr/bin/env python3
"""Which Normal-mode keys are taken, in vim, in helix, and here.

Three columns, each read from the authority rather than from memory:

* **vim** — nvim's own ``$VIMRUNTIME/doc/index.txt``, section ``*normal-index*``.
  That table marks its unused keys itself (``not used``), so the empty cells
  are vim's statement, not ours.
* **helix** — ``helix-term/src/keymap/default.rs``, the ``"Normal mode"``
  trie, top level only.
* **yumete** — the one ``match key`` in ``Editor::on_normal_key``, plus the
  ``KeyPreset::Vim`` table, which rebinds a handful of keys on top of it.

Run it; do not copy its output into a document. The point of a generator is
that the answer cannot go stale.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

KEYS = [chr(c) for c in range(0x21, 0x7F)]


def vim_runtime() -> Path:
    """Ask nvim where its runtime is, and fall back to the cellar."""
    try:
        out = subprocess.run(
            ["nvim", "--clean", "--headless", "+echo $VIMRUNTIME", "+q"],
            capture_output=True, text=True, timeout=20, check=False,
        )
        path = Path((out.stderr or out.stdout).strip())
        if (path / "doc" / "index.txt").exists():
            return path
    except (OSError, subprocess.SubprocessError):
        pass
    cellar = sorted(Path("/opt/homebrew/Cellar/neovim").glob("*/share/nvim/runtime"))
    if cellar:
        return cellar[-1]
    sys.exit("no nvim runtime: install neovim, or pass --vim-index")


def read_vim(index: Path) -> dict[str, str]:
    """The ``*normal-index*`` table, by the character it starts with."""
    lines = index.read_text(encoding="utf-8", errors="replace").splitlines()
    first = next(i for i, l in enumerate(lines) if "*normal-index*" in l)
    rest = lines[first + 1:]
    last = next(i for i, l in enumerate(rest) if l.startswith("====")) + first + 1
    taken: dict[str, str] = {}
    for line in lines[first:last]:
        if not line.startswith(("|", "\t")) or "\t" not in line.lstrip("|"):
            continue
        cols = [c for c in line.split("\t") if c]
        if len(cols) < 2:
            continue
        char, what = cols[-2], cols[-1].strip()
        if line.startswith("|"):
            if len(cols) < 3:
                continue
            char, what = cols[-2], cols[-1].strip()
        # Strip the register prefix and a trailing argument placeholder.
        char = re.sub(r'^\["x\]', "", char).strip()
        if len(char) > 2 and char.startswith('"') and char.endswith('"'):
            char = char[1:-1]
        char = re.sub(r"^\{[^}]*\}", "", char)
        char = re.sub(r"\{[^{}]*\}$", "", char)
        if not char or char[0] not in KEYS:
            continue
        what = re.sub(r"^[12]\s+", "", what)
        if what.startswith("not used"):
            continue
        taken.setdefault(char[0], what)
    return taken


def read_helix(default_rs: Path) -> dict[str, str]:
    """The top level of helix's Normal-mode trie."""
    text = default_rs.read_text(encoding="utf-8")
    start = text.index('keymap!({ "Normal mode"')
    depth, end = 0, start
    for i, ch in enumerate(text[start:], start):
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                end = i
                break
    taken: dict[str, str] = {}
    depth = 0
    for line in text[start:end].splitlines()[1:]:
        if depth == 0:
            quoted = r'"(?:\\.|[^"\\])+"'
            m = re.match(rf'\s*((?:{quoted}\s*\|\s*)*{quoted})\s*=>\s*(.+?),?\s*$', line)
            if m:
                names = [n.replace('\\"', '"').replace("\\\\", "\\")
                         for n in re.findall(rf'{quoted}', m.group(1))]
                names = [n[1:-1] if n.startswith('"') else n for n in names]
                what = m.group(2).rstrip("{").strip() or "(submenu)"
                if what.startswith('{ "'):
                    what = re.match(r'\{\s*"([^"]+)"', what).group(1) + " 菜單"
                for name in names:
                    if len(name) == 1 and name in KEYS:
                        taken.setdefault(name, what)
        depth += line.count("{") - line.count("}")
    return taken


def read_yumete(keys_rs: Path, keymap_rs: Path) -> tuple[dict[str, str], dict[str, str]]:
    """The top-level ``match key``, and what the vim preset lays over it."""
    text = keys_rs.read_text(encoding="utf-8")
    start = text.index("    fn on_normal_key(&mut self, key: Key) {")
    body = text[start:text.index("\n    /// Hand a half-finished key", start)]
    at = body.rindex("\n        match key {")
    arms = body[at:].splitlines()[2:]
    def code(line: str) -> str:
        """Braces inside `\'{\'`, a string or a comment are not Rust braces."""
        line = re.sub(r"'(\\.|[^'])'", "''", line)
        line = re.sub(r'"(\\.|[^"])*"', '""', line)
        return line.split("//")[0]

    taken: dict[str, str] = {}
    depth, pattern, chars, body_of, held = 0, "", [], [], ""
    skip = {"repeat", "current_buffer", "key_preset", "rope", "sel", "head"}

    def settle() -> None:
        """Name the arm by the first real call in it, not by `repeat`."""
        text = " ".join(body_of)
        what = next(
            (m for m in re.findall(r"(?:self|e)\.([a-z_]{3,})\(", text) if m not in skip),
            "",
        )
        if not what:
            m = re.search(r"pending = Pending::(\w+)", text) or re.search(r"\b([a-z_]{4,})\(", text)
            what = m.group(1) if m else "…"
        for c in chars:
            taken.setdefault(c, what + held)

    for line in arms:
        if depth == 0:
            if chars:
                settle()
                chars, body_of = [], []
            if re.match(r"            [A-Z_]", line):
                pattern = ""
            if line.startswith("            Key::") or pattern:
                pattern += " " + line.strip()
                if "=>" in pattern:
                    head, _, tail = pattern.partition("=>")
                    guard = " 有條件" if re.search(r"\bif\b", head) else ""
                    for c in re.findall(r"Key::Char\('(\\?.)'\)", head):
                        c = {"\\'": "'", "\\\\": "\\"}.get(c, c)
                        if c in KEYS:
                            chars.append(c)
                    body_of, held, pattern = [tail], guard, ""
        else:
            body_of.append(line.strip())
        depth += code(line).count("{") - code(line).count("}")
        if depth < 0:
            break
    if chars:
        settle()
    text = keymap_rs.read_text(encoding="utf-8")
    table = text[text.index("KeyPreset::Vim => &["):]
    table = table[:table.index("\n            ],")]
    preset = {k: v for k, v in re.findall(r'\("(.)", "([^"]*)"\)', table)}
    return taken, preset


def main() -> int:
    root = Path(__file__).resolve().parent.parent
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--vim-index", type=Path)
    ap.add_argument("--helix", type=Path, default=Path.home() / "Programs/helix/helix-term/src/keymap/default.rs")
    ap.add_argument("--free", action="store_true", help="只列三邊都空着的鍵")
    args = ap.parse_args()

    index = args.vim_index or (vim_runtime() / "doc" / "index.txt")
    vim = read_vim(index)
    helix = read_helix(args.helix) if args.helix.exists() else {}
    if not helix:
        print(f"note: {args.helix} 不在，helix 那一欄是空的", file=sys.stderr)
    ours, preset = read_yumete(
        root / "crates/yumete-core/src/editor/keys.rs",
        root / "crates/yumete-cjk/src/keymap.rs",
    )

    print(f"vim: {index}")
    print(f"helix: {args.helix}")
    print("yumete: crates/yumete-core/src/editor/keys.rs ＋ crates/yumete-cjk/src/keymap.rs")
    print()
    free = []
    if not args.free:
        print("| 鍵 | vim | helix | yumete | yumete vim 預設 |")
        print("| --- | --- | --- | --- | --- |")
    for c in KEYS:
        row = (vim.get(c, ""), helix.get(c, ""), ours.get(c, ""), preset.get(c, ""))
        if not any(row):
            free.append(c)
        if args.free:
            continue
        key = "`` ` ``" if c == "`" else f"`{c}`".replace("|", "\\|")
        print(f"| {key} | " + " | ".join(x.replace("|", "\\|") for x in row) + " |")
    print()
    spell = lambda d: " ".join(f"`{c}`" for c in KEYS if c not in d) or "（沒有）"
    print(f"vim 空着的：{spell(vim)}")
    print(f"helix 空着的：{spell(helix)}")
    print(f"yumete 空着的：{spell(ours)}")
    print(f"yumete 空着的（vim 預設鋪完之後）：{spell({**ours, **preset})}")
    print()
    print("`0`-`9` 三邊都是計數，不是真空；`\\` 是 vim 自己寫明留給使用者的那一個。")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
