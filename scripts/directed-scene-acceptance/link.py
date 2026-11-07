"""The link between a binary under test, the head it was built from, its runtime bundle and the scripts that judge it.

An acceptance run is only about the binary it ran. `harness.binary_info` records a sha256 and an mtime, and `older_than_sources`
compares the mtime with `crates/**/*.rs`: that does not tie the binary to a head (a newer binary built from another worktree or
another head passes it, and `runtime/dist` was only recorded). This module adds the attestation:

    python3 scripts/directed-scene-acceptance/link.py create --render target/debug/tessel-render    # right after the build
    python3 scripts/directed-scene-acceptance/link.py check  --render target/debug/tessel-render

`create` refuses unless the working tree is clean, every tracked file under `crates/` and `Cargo.*` is older than the binary and
every tracked file under `runtime/` is older than `runtime/dist`; it then writes `<binary>.link.json` (untracked, next to the
binary) holding the head, the git tree ids of the build inputs (`crates`, `runtime`, `scripts`, `Cargo.toml`, `Cargo.lock`,
`rust-toolchain.toml`), the sha256 of the binary and of `runtime/dist/*`, and the toolchain versions. `check` recomputes all of
that from the repository and the files on disk, and the harness refuses a binary whose link does not hold (a BLOCKED phase, or an
informational, non-accepting run under `--allow-stale-binary`).

This is an attestation by whoever ran `create` right after the build, verified afterwards: it cannot prove that the build used
those inputs (a binary swapped before `create` would be attested), only that nothing changed since, and that the binary and the
bundle on disk are the ones attested for exactly these inputs. It is not a reproducible-build proof.
"""
import argparse
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import time

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent.parent
INPUTS = ["crates", "runtime", "scripts", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml"]
SCHEMA = "s10-link/1"
RUNTIME_DIST = ROOT / "runtime" / "dist"
DIST_FILES = ("runtime.js", "index.html")


def git(*args):
    run = subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True)
    return run.stdout.strip() if run.returncode == 0 else None


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def tree_ids(rev="HEAD"):
    return {name: git("rev-parse", f"{rev}:{name}") for name in INPUTS}


def dirty_files():
    out = git("status", "--porcelain")
    return None if out is None else [line for line in out.splitlines() if line]


def link_path(binary):
    return pathlib.Path(str(binary) + ".link.json")


def dist_hashes(dist=RUNTIME_DIST):
    return {name: sha256_file(dist / name) for name in DIST_FILES if (dist / name).is_file()}


def tool_versions():
    def out(*cmd):
        try:
            return subprocess.run(cmd, capture_output=True, text=True).stdout.strip()
        except OSError:
            return None
    return {"rustc": out("rustc", "-V"), "node": out("node", "--version")}


def newest_tracked_mtime(paths):
    """Newest mtime among the git-tracked files below `paths` (0.0 when none)."""
    listed = git("ls-files", "-z", "--", *paths)
    newest = 0.0
    for name in (listed or "").split("\0"):
        if name and (ROOT / name).is_file():
            newest = max(newest, os.path.getmtime(ROOT / name))
    return newest


def create(binaries, now=None):
    """Writes the link of `binaries` (`{key: path}`); returns `(ok, reasons)`."""
    reasons = []
    dirty = dirty_files()
    if dirty is None or dirty:
        reasons.append(f"the working tree is not clean ({'git failed' if dirty is None else len(dirty)} entries): a link describes a commit")
    head = git("rev-parse", "HEAD")
    if not head:
        reasons.append("git cannot name HEAD")
    sources = newest_tracked_mtime(["crates", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml"])
    entries = {}
    for key, path in binaries.items():
        path = pathlib.Path(path)
        if not path.is_file():
            reasons.append(f"{path} is not a file")
            continue
        if os.path.getmtime(path) < sources:
            reasons.append(f"{path} is older than a tracked source of the build: rebuild before linking")
        entries[key] = {"path": str(path), "sha256": sha256_file(path), "bytes": path.stat().st_size}
    dist = dist_hashes()
    if set(dist) != set(DIST_FILES):
        reasons.append(f"runtime/dist lacks {sorted(set(DIST_FILES) - set(dist))}: run ./scripts/runtime setup")
    else:
        runtime_sources = newest_tracked_mtime(["runtime"])
        if min(os.path.getmtime(RUNTIME_DIST / name) for name in DIST_FILES) < runtime_sources:
            reasons.append("runtime/dist is older than a tracked file of runtime/: rebuild the bundle before linking")
    if reasons:
        return False, reasons
    document = {"schema": SCHEMA, "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(now or time.time())), "head": head,
                "inputs": tree_ids(head), "binaries": entries, "runtime_dist": dist, "tools": tool_versions(),
                "about": "attestation after the build, verified by link.py check; not a reproducible-build proof"}
    for key, entry in entries.items():
        link_path(entry["path"]).write_text(json.dumps(document, indent=1) + "\n", encoding="utf-8")
    return True, []


def check(binary, key="render", dist=RUNTIME_DIST):
    """`(ok, reasons, info)`: does the link of `binary` hold for the repository's HEAD and the files on disk?"""
    binary = pathlib.Path(binary)
    path = link_path(binary)
    if not path.is_file():
        return False, [f"{path.name} does not exist: run link.py create right after building"], {}
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        return False, [f"{path.name} is unreadable: {error}"], {}
    reasons = []
    if document.get("schema") != SCHEMA:
        reasons.append(f"unknown link schema {document.get('schema')!r}")
    entry = document.get("binaries", {}).get(key)
    if not entry:
        reasons.append(f"the link does not name a `{key}` binary")
    elif not binary.is_file() or entry.get("sha256") != sha256_file(binary):
        reasons.append("the binary on disk is not the one attested (sha256 differs)")
    held = tree_ids("HEAD")
    if any(value is None for value in held.values()) or document.get("inputs") != held:
        reasons.append("the build inputs (crates, runtime, scripts, Cargo.*, toolchain) at HEAD differ from the attested ones: "
                       + ", ".join(sorted(k for k in held if document.get("inputs", {}).get(k) != held[k])))
    if dist_hashes(dist) != document.get("runtime_dist") or not document.get("runtime_dist"):
        reasons.append("runtime/dist is not the bundle attested with this binary")
    info = {"link": str(path), "attested_head": document.get("head"), "head": git("rev-parse", "HEAD"), "created_utc": document.get("created_utc"),
            "inputs": document.get("inputs"), "runtime_dist": document.get("runtime_dist"), "tools": document.get("tools")}
    return not reasons, reasons, info


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", choices=["create", "check"])
    parser.add_argument("--render", default=str(ROOT / "target/debug/tessel-render"))
    args = parser.parse_args(argv)
    if args.command == "create":
        ok, reasons = create({"render": args.render})
    else:
        ok, reasons, _ = check(args.render)
    for reason in reasons:
        print(reason, file=sys.stderr)
    print("link holds" if ok and args.command == "check" else "link written" if ok else "link refused")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
