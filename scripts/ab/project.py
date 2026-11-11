"""What a project document says and the identity of a run: sequence, cuts, source labels, hashes, git state.

Nothing here carries a name or a path of the project: sources are published as labels, kinds, sizes and a head/tail hash."""
import glob
import hashlib
import os
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


def parse_project(document):
    """What the experiment needs from a project document, without a name or a path: sequence, cuts, source locations."""
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
    sources = []
    for entry in (document.get("library") or {}).get("assets", []):
        source = (entry.get("asset") or {}).get("source") or {}
        if source.get("type") == "file" and source.get("path"):
            sources.append({"kind": entry.get("kind"), "path": source["path"]})
        elif source.get("type") == "frame_sequence" and source.get("pattern"):
            sources.append({"kind": entry.get("kind"), "pattern": source["pattern"]})
    return {
        "length": sequence["length"], "fps": fps, "cuts": cuts, "video_starts": video_starts,
        "clip_kinds": {kind: sum(1 for c in clips if c.get("kind") == kind) for kind in sorted({c.get("kind") for c in clips})},
        "sources": sources,
    }


def head_tail_hash(path, chunk=1 << 20):
    """sha256 of the first and last MiB plus the size: identity enough to notice another file, without reading 66 MiB."""
    digest = hashlib.sha256()
    size = os.path.getsize(path)
    with open(path, "rb") as handle:
        digest.update(handle.read(chunk))
        if size > 2 * chunk:
            handle.seek(size - chunk)
            digest.update(handle.read(chunk))
    return size, digest.hexdigest()


def check_sources(sources):
    """Presence and identity of every source, WITHOUT its name: a label, kind, size and a head/tail hash."""
    out = []
    for source in sources:
        if "path" in source:
            exists = os.path.isfile(source["path"])
            entry = {"label": label_of(source["path"]), "kind": source["kind"], "present": exists}
            if exists:
                entry["size"], entry["head_tail_sha256"] = head_tail_hash(source["path"])
        else:
            pattern = re.sub(r"%0?\d*d", "*", source["pattern"])
            found = glob.glob(pattern)
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
    """Copies only the document into a private folder (assets are absolute paths, read in place)."""
    os.makedirs(work, exist_ok=True)
    copy = os.path.join(work, "project.tessel.json")
    shutil.copyfile(document, copy)
    return copy
