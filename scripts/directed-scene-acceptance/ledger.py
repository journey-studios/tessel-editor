"""The amendment ledger of S10: what the oracle, the verifiers, the controls, the corpus and the tolerances were, and when they changed.

`amendments.json` is the one book the report prints. Every file that DECIDES a verdict is pinned (`PINNED`): the tolerances, the
oracle, the corpus generator and its `expected.json`, the verifiers, the controls, the driver (`proof.py`, which picks frames,
gains and exit codes), the binary link, the sensitivity analysis, this ledger, and the read-only helpers whose code runs inside the
verdict (`directed-scene-proof/model.py` and `harness.py`, `directed-scene-export-proof/export_verify.py` and `export_corpus.py`).
Every change to a pinned file after the baseline is a round with a reason, the finding that motivated it, the SHA-256 of every
pinned file before and after, and the amendments it contains.

`tolerances_frozen` fails when:
  * the ledger names another baseline than `ANCHOR` (the head of PR #8 that the first independent review read), or `ANCHOR` is not an
    ancestor of HEAD, or the baseline hashes are not what Git holds at `ANCHOR`;
  * a pinned file differs from the last recorded `after`, rounds do not chain, a changed file is named by no amendment, or the set of
    pinned files is not the one the ledger and this module agree on;
  * the book is not append-only against Git: every committed version of `amendments.json` reachable from HEAD must be a prefix of
    the current one, so `rounds = []` (or a rewritten round) is caught even if the baseline and the hashes are made to match.

What this is NOT: the ledger is the author's bookkeeping, and `ANCHOR` is a constant in a file the author can edit; the real anchor is the
Git history that this module reads. It proves which edits were recorded and that the record was not rewritten in published commits,
not that none preceded the record, and not that the numbers are right.

    python3 scripts/directed-scene-acceptance/ledger.py hashes      # current SHA-256 of the pinned files
    python3 scripts/directed-scene-acceptance/ledger.py seal R2     # after editing: writes `after` of round R2 (author's act, same commit)
"""
import hashlib
import json
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent.parent
LEDGER = HERE / "amendments.json"
ANCHOR = "a8480074788dd8ee435c9e772b1ff44d067279ae"
BASELINE_PINNED = ["tolerances.json", "oracle.py", "corpus.py", "verify.py", "controls.py", "corpus/expected.json"]
PINNED = BASELINE_PINNED + [
    "proof.py", "link.py", "sensitivity.py", "ledger.py",
    "../directed-scene-proof/model.py", "../directed-scene-proof/harness.py",
    "../directed-scene-export-proof/export_verify.py", "../directed-scene-export-proof/export_corpus.py",
]


def sha256_bytes(data):
    return hashlib.sha256(data).hexdigest()


def current(names=None):
    return {name: sha256_bytes((HERE / name).read_bytes()) for name in (names or PINNED)}


def load():
    return json.loads(LEDGER.read_text(encoding="utf-8"))


def _git(*args):
    run = subprocess.run(["git", *args], cwd=ROOT, capture_output=True)
    return run.stdout if run.returncode == 0 else None


def _relative(name):
    return str((HERE / name).resolve().relative_to(ROOT))


def git_blob_hashes(commit, names=None):
    """SHA-256 of the pinned files as Git holds them at `commit`; None for a file or commit Git cannot show."""
    out = {}
    for name in names or BASELINE_PINNED:
        shown = _git("show", f"{commit}:{_relative(name)}")
        out[name] = sha256_bytes(shown) if shown is not None else None
    return out


def is_ancestor(commit, descendant="HEAD"):
    return subprocess.run(["git", "merge-base", "--is-ancestor", commit, descendant], cwd=ROOT, capture_output=True).returncode == 0


def committed_versions():
    """`[(commit, ledger dict)]` of every committed version of the book reachable from HEAD (newest first); None when Git cannot say.

    A shallow clone (a CI checkout with `--depth`) cannot say: `git log` stops silently at the boundary, so a side branch that rewrote
    the book would simply be missing from the list. Anything but a plain `false` from Git (a shallow repository, an error, an old Git that
    does not know the question) is refused, and `verify_ledger` reports the history as unreadable."""
    shallow = _git("rev-parse", "--is-shallow-repository")
    if shallow is None or shallow.strip() != b"false":
        return None
    listed = _git("log", "--full-history", "--format=%H", "--", _relative("amendments.json"))
    if listed is None:
        return None
    versions = []
    for commit in listed.decode().split():
        shown = _git("show", f"{commit}:{_relative('amendments.json')}")
        if shown is None:
            continue
        try:
            versions.append((commit, json.loads(shown)))
        except ValueError:
            versions.append((commit, {"rounds": "unreadable"}))
    return versions


def effective_pinned(ledger):
    pinned = list(ledger.get("pinned", []))
    for round_ in ledger.get("rounds", []):
        pinned += [name for name in round_.get("pins", []) if name not in pinned]
    return pinned


def verify_ledger(ledger=None, now=None, git_hashes=git_blob_hashes, ancestor=is_ancestor, versions=committed_versions):
    """`[(check, ok, detail)]` for the ledger against the files on disk (`now`), Git at the anchor and the committed history."""
    ledger = ledger if ledger is not None else load()
    now = now if now is not None else current()
    checks = []
    baseline = ledger.get("baseline", {})
    commit = baseline.get("commit", "")
    checks.append(("the baseline is the anchor fixed in ledger.py (the head of PR #8 read by the first independent review)", commit == ANCHOR, (commit, ANCHOR)))
    checks.append(("the anchor is an ancestor of HEAD", bool(commit) and ancestor(commit), commit))
    held = git_hashes(commit) if commit else {}
    checks.append(("the baseline hashes are exactly what Git holds at the baseline commit", bool(held) and held == baseline.get("hashes"),
                   {k: (held.get(k), v) for k, v in baseline.get("hashes", {}).items() if held.get(k) != v}))
    checks.append(("the ledger pins the baseline files, and baseline plus the files pinned by rounds is exactly the pinned set of ledger.py",
                   ledger.get("pinned") == BASELINE_PINNED and sorted(baseline.get("hashes", {})) == sorted(BASELINE_PINNED)
                   and sorted(effective_pinned(ledger)) == sorted(PINNED), effective_pinned(ledger)))
    previous = dict(baseline.get("hashes", {}))
    rounds = ledger.get("rounds", [])
    seen = set(BASELINE_PINNED)
    for round_ in rounds:
        name = f"round {round_.get('id')}"
        before, after = round_.get("before", {}), round_.get("after", {})
        pins = [p for p in round_.get("pins", []) if p not in seen]
        checks.append((f"{name}: starts from the previous recorded state", before == previous,
                       {k: (previous.get(k), before.get(k)) for k in set(previous) | set(before) if previous.get(k) != before.get(k)}))
        checks.append((f"{name}: records a reason and at least one amendment with a motivating finding",
                       bool(round_.get("reason")) and bool(round_.get("amendments")) and all(a.get("finding") and a.get("change") for a in round_["amendments"]), round_.get("id")))
        seen |= set(pins)
        checks.append((f"{name}: records the hash of exactly the pinned files so far", sorted(after) == sorted(seen), sorted(set(after) ^ seen)))
        changed = sorted(k for k in seen if before.get(k) != after.get(k))
        named = all(any(k in a.get("files", []) for a in round_.get("amendments", [])) for k in changed)
        checks.append((f"{name}: changed or newly pinned {len(changed)} file(s), each named by an amendment", bool(changed) and named, changed))
        previous = after
    checks.append(("every pinned file equals the last recorded state (no unrecorded edit to anything that decides a verdict)",
                   previous == now, {k: (previous.get(k), now.get(k)) for k in set(previous) | set(now) if previous.get(k) != now.get(k)}))
    history = versions()
    if history is None:
        checks.append(("Git history of the book is readable (append-only check)", False, "git log failed"))
    else:
        bad = []
        for commit_id, old in history:
            old_rounds = old.get("rounds")
            if not isinstance(old_rounds, list) or old_rounds != rounds[:len(old_rounds)] or old.get("baseline") != baseline:
                bad.append(commit_id[:12])
        checks.append((f"the book only grows: each of the {len(history)} committed version(s) reachable from HEAD is a prefix of this one (same baseline, same rounds)",
                       not bad, bad[:5]))
    return checks


def numeric_leaves(node, prefix=""):
    """Dotted paths of every number (or list of numbers) in the tolerances, outside the prose blocks."""
    out = []
    for key, value in node.items():
        if not prefix and key in ("justifications", "provenance", "blockedPolicy"):
            continue
        path = f"{prefix}.{key}" if prefix else key
        if isinstance(value, dict):
            out += numeric_leaves(value, path)
        elif isinstance(value, bool):
            continue
        elif isinstance(value, (int, float)):
            out.append(path)
        elif isinstance(value, list) and value and all(isinstance(x, (int, float)) or (isinstance(x, list) and all(isinstance(y, (int, float)) for y in x)) for x in value):
            out.append(path)
    return out


def justification_gaps(tolerances):
    """`(numbers without a justification, justifications of no number)`; each justification names a basis and a text."""
    justifications = tolerances.get("justifications", {})
    leaves = numeric_leaves(tolerances)
    unjustified = [path for path in leaves if path not in justifications]
    stale = [path for path in justifications if path not in leaves]
    malformed = [path for path, entry in justifications.items()
                 if entry.get("basis") not in ("analytic", "inherited", "choice") or not str(entry.get("text", "")).strip()]
    return unjustified, stale, malformed


def main(argv):
    if argv[:1] == ["hashes"]:
        print(json.dumps(current(), indent=1))
        return 0
    if argv[:1] == ["seal"] and len(argv) == 2:
        ledger = load()
        target = next((r for r in ledger["rounds"] if r["id"] == argv[1]), None)
        if target is None or target is not ledger["rounds"][-1]:
            print("only the last round can be sealed", file=sys.stderr)
            return 2
        target["after"] = current()
        LEDGER.write_text(json.dumps(ledger, indent=1) + "\n", encoding="utf-8")
        return 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
