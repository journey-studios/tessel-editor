"""The amendment ledger of S10: what the oracle, the verifier, the controls, the corpus and the tolerances were, and when they changed.

`amendments.json` is the one book the report prints. Every change to a *pinned* file (the tolerances, the oracle, the corpus
generator and its `expected.json`, the verifiers, the controls) after the baseline is a round with a reason, the finding that
motivated it, the SHA-256 of every pinned file before and after, and the amendments it contains. The harness fails
(`tolerances_frozen`) when a pinned file differs from the last recorded `after`, when rounds do not chain, or when the baseline
does not match what Git holds for it. This is bookkeeping, not pre-registration: it proves *which* edits were recorded, not that
none preceded the record.

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
PINNED = ["tolerances.json", "oracle.py", "corpus.py", "verify.py", "controls.py", "corpus/expected.json"]


def sha256_bytes(data):
    return hashlib.sha256(data).hexdigest()


def current():
    return {name: sha256_bytes((HERE / name).read_bytes()) for name in PINNED}


def load():
    return json.loads(LEDGER.read_text())


def git_blob_hashes(commit):
    """SHA-256 of the pinned files as Git holds them at `commit`; None for a file or commit Git cannot show."""
    out = {}
    for name in PINNED:
        shown = subprocess.run(["git", "show", f"{commit}:{(HERE / name).relative_to(ROOT)}"], cwd=ROOT, capture_output=True)
        out[name] = sha256_bytes(shown.stdout) if shown.returncode == 0 else None
    return out


def verify_ledger(ledger=None, now=None, git_hashes=git_blob_hashes):
    """`[(check, ok, detail)]` for the ledger against the files on disk (`now`) and Git (`git_hashes(commit)`)."""
    ledger = ledger if ledger is not None else load()
    now = now if now is not None else current()
    checks = []
    baseline = ledger.get("baseline", {})
    held = git_hashes(baseline.get("commit", "")) if baseline.get("commit") else {}
    checks.append(("the baseline hashes are exactly what Git holds at the baseline commit", bool(held) and held == baseline.get("hashes"),
                   {k: (held.get(k), v) for k, v in baseline.get("hashes", {}).items() if held.get(k) != v}))
    checks.append(("the ledger pins exactly the pinned files", sorted(baseline.get("hashes", {})) == sorted(PINNED) and ledger.get("pinned") == PINNED, ledger.get("pinned")))
    previous = baseline.get("hashes", {})
    rounds = ledger.get("rounds", [])
    for round_ in rounds:
        checks.append((f"round {round_['id']}: starts from the previous recorded state", round_.get("before") == previous,
                       {k: (previous.get(k), v) for k, v in round_.get("before", {}).items() if previous.get(k) != v}))
        checks.append((f"round {round_['id']}: records a reason and at least one amendment with a motivating finding",
                       bool(round_.get("reason")) and bool(round_.get("amendments")) and all(a.get("finding") and a.get("change") for a in round_["amendments"]), round_["id"]))
        changed = sorted(k for k in PINNED if round_.get("before", {}).get(k) != round_.get("after", {}).get(k))
        checks.append((f"round {round_['id']}: changed {len(changed)} pinned file(s), each named by an amendment",
                       bool(changed) and all(any(k in a.get("files", []) for a in round_["amendments"]) for k in changed), changed))
        previous = round_.get("after", {})
    checks.append(("every pinned file equals the last recorded state (no unrecorded edit to the oracle, verifier, controls, corpus or tolerances)",
                   previous == now, {k: (previous.get(k), now.get(k)) for k in PINNED if previous.get(k) != now.get(k)}))
    return checks


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
        LEDGER.write_text(json.dumps(ledger, indent=1) + "\n")
        return 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
