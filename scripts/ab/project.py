"""What a project document says and the identity of a run: sequence, cuts, source labels, hashes, git state.

Nothing here carries a name or a path of the project: sources are published as labels, kinds, sizes and a head/tail hash."""
import glob
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess

from ab import ROOT


def label_of(text):
    """The short stable label a trace and this report use for a private name (FNV-1a, 32 bits)."""
    value = 0xCBF29CE484222325
    for byte in text.encode():
        value ^= byte
        value = (value * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"{value & 0xFFFFFFFF:08x}"


def absolute(base, stored):
    """A stored source path the way the editor reads it (`decode_path`, crates/core/src/document.rs): an absolute one as it is, a relative one under the folder of
    the document (never the working directory). None for a relative one that leaves that folder or is empty: the editor refuses it."""
    if os.path.isabs(stored):
        return stored
    if base is None or not stored or ".." in pathlib.PurePath(stored).parts:
        return None
    return os.path.join(base, stored)


def library_sources(document):
    """(entry, source, key) of every file or frame-sequence source of the library, `key` being where its location is (`path` / `pattern`): the one place that
    knows where a document keeps them."""
    for entry in (document.get("library") or {}).get("assets", []):
        source = (entry.get("asset") or {}).get("source") or {}
        key = {"file": "path", "frame_sequence": "pattern"}.get(source.get("type"))
        if key and source.get(key):
            yield entry, source, key


def source_of(entry, source, key, base):
    """The `{kind, path|pattern}` of one source; `outside` marks a relative location the editor refuses."""
    resolved = absolute(base, source[key])
    return {"kind": entry.get("kind"), key: source[key] if resolved is None else resolved, **({"outside": True} if resolved is None else {})}


def parse_project(document, base=None):
    """What the experiment needs from a project document, without a name or a path: sequence, cuts, source locations. `base` is the folder of the document:
    the relative paths of its managed media (stored relative to it) resolve there, as the editor does."""
    sequences = document.get("sequences") or []
    active = document.get("active_sequence")
    sequence = next((s for s in sequences if s.get("id") == active), sequences[0] if sequences else None)
    if sequence is None:
        raise ValueError("the document has no sequence")
    rate = sequence["rate"]
    fps = rate["num"] / rate["den"]
    clips = sequence.get("clips") or []
    cuts = sorted({c["start"] for c in clips if c.get("kind") == "scene" and c["start"] > 0})
    video_starts = sorted({c["start"] for c in clips if c.get("kind") == "video" and c["start"] > 0})
    sources = [source_of(entry, source, key, base) for entry, source, key in library_sources(document)]
    return {
        "length": sequence["length"], "fps": fps, "cuts": cuts, "video_starts": video_starts,
        "clip_kinds": {kind: sum(1 for c in clips if c.get("kind") == kind) for kind in sorted({c.get("kind") for c in clips})},
        "sources": sources,
    }


def head_tail_hash(path, chunk=1 << 20):
    """sha256 of the first and last MiB plus the size: identity enough to notice another file, without reading 66 MiB. The tail starts where the head
    ends, so a file between one and two chunks has every byte read once and one that fits in a chunk is read whole."""
    digest = hashlib.sha256()
    size = os.path.getsize(path)
    with open(path, "rb") as handle:
        digest.update(handle.read(chunk))
        if size > chunk:
            handle.seek(max(chunk, size - chunk))
            digest.update(handle.read(chunk))
    return size, digest.hexdigest()


def check_sources(sources):
    """Presence and identity of every source, WITHOUT its name: a label, kind, size and a head/tail hash."""
    out = []
    for source in sources:
        if "path" in source:
            exists = not source.get("outside") and os.path.isfile(source["path"])
            entry = {"label": label_of(source["path"]), "kind": source["kind"], "present": exists}
            if exists:
                entry["size"], entry["head_tail_sha256"] = head_tail_hash(source["path"])
        else:
            pattern = re.sub(r"%0?\d*d", "*", source["pattern"])
            found = [] if source.get("outside") else glob.glob(pattern)
            entry = {"label": label_of(source["pattern"]), "kind": source["kind"], "present": bool(found), "frames": len(found)}
        out.append(entry)
    return out


# --------------------------------------------------------------------------------------- machine and the editor
def git_state():
    def run(*args):
        return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    diff = run("diff", "HEAD")
    return {"sha": run("rev-parse", "HEAD"), "dirty_files": len([l for l in run("status", "--porcelain").splitlines() if l]),
            "diff_sha256": hashlib.sha256(diff.encode()).hexdigest() if diff else None}


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def new_workspace_copy(document, work):
    """Copies the document into a private folder. Assets are read in place: the paths a document stores relative to its own folder are written into the copy
    as absolute ones (the copy lives elsewhere); a document that stores none is copied byte for byte."""
    os.makedirs(work, exist_ok=True)
    copy = os.path.join(work, "project.tessel.json")
    base = os.path.dirname(os.path.abspath(document))
    data = json.loads(pathlib.Path(document).read_text())
    changed = False
    for _, source, key in library_sources(data):
        resolved = absolute(base, source[key])
        if not os.path.isabs(source[key]) and resolved is not None:  # one the editor refuses stays as it is: the copy is refused the same way
            source[key], changed = resolved, True
    if changed:
        pathlib.Path(copy).write_text(json.dumps(data))
    else:
        shutil.copyfile(document, copy)
    return copy
