"""Turns a selection + rules into a rename plan and executes it safely.

Paths in a plan are relative to the folder the user is browsing ("base").
Execution happens in phases: first all files, then folders deepest-first,
because renaming a folder changes the paths of everything inside it. Each
phase renames in two steps (to temporary names, then to the targets) so swaps
and case-only changes work, and rolls back if anything fails.
"""
from __future__ import annotations

import errno
import os
import shutil
import uuid
from concurrent.futures import ThreadPoolExecutor
from typing import Dict, List, Optional, Tuple

from meta import cache as meta_cache
from rules import RuleError, apply_rules, needs_meta, split_name, validate_name
from i18n import t

DEFAULT_SIDECARS = "xmp,srt,ass,ssa,sub,idx,vtt,nfo,lrc,cue,aae,thm"

pool = ThreadPoolExecutor(max_workers=8)

Pair = Tuple[str, str]


class PlanError(Exception):
    pass


def _parent(rel: str) -> str:
    d = os.path.dirname(rel)
    return d


def _join(*parts: str) -> str:
    return "/".join(p for p in parts if p)


def compute_plan(base: str, items: List[str], rules: List[dict], options: dict) -> List[dict]:
    """Return plan rows: {old, new, changed, error, dir, sidecar}."""
    allow_subdirs = bool(options.get("allow_subdirs"))
    conflict_mode = options.get("conflict", "error")
    folder_name = os.path.basename(base)
    parent_name = os.path.basename(os.path.dirname(base))

    real_base = os.path.realpath(base)
    for it in items:
        parts = it.split("/")
        if not it or it.startswith("/") or any(p in ("", ".", "..") for p in parts):
            raise PlanError(t("Ungültiger Pfad: {path}", path=it))
        # a symlinked folder on the way must not lead out of the opened folder
        parent = os.path.realpath(os.path.join(base, os.path.dirname(it)))
        if os.path.commonpath([parent, real_base]) != real_base:
            raise PlanError(t("Pfad führt aus dem Ordner heraus: {path}", path=it))

    # Stat everything first; metadata is loaded lazily per file and in parallel
    stats: Dict[str, Optional[os.stat_result]] = {}
    for it in items:
        try:
            stats[it] = os.stat(os.path.join(base, it))
        except OSError:
            stats[it] = None
    metas: Dict[str, dict] = {}
    if needs_meta(rules):
        files = [it for it in items if stats[it] and not os.path.isdir(os.path.join(base, it))]
        for it, m in zip(files, pool.map(lambda p: meta_cache.meta(os.path.join(base, p)), files)):
            metas[it] = m

    # With sidecars on, a selected "movie.de.srt" follows "movie.mkv" instead of
    # going through the rules itself (and does not take a number in sequences).
    followers: set = set()
    if options.get("sidecars"):
        exts = _sidecar_exts(options.get("sidecar_exts") or DEFAULT_SIDECARS)
        mains: Dict[str, List[str]] = {}
        for it in items:
            if stats[it] and split_name(os.path.basename(it))[1].lower() not in exts:
                mains.setdefault(_parent(it), []).append(split_name(os.path.basename(it))[0])
        for it in items:
            name = os.path.basename(it)
            if split_name(name)[1].lower() in exts and any(
                    name.startswith(stem + ".") for stem in mains.get(_parent(it), [])):
                followers.add(it)
    items = [it for it in items if it not in followers]

    folder_counter: Dict[str, int] = {}
    plan: List[dict] = []
    for i, it in enumerate(items):
        st = stats[it]
        parent = _parent(it)
        fname = os.path.basename(it)
        if st is None:
            plan.append({"old": it, "new": it, "changed": False, "dir": False,
                         "error": t("Datei nicht gefunden")})
            continue
        is_dir = os.path.isdir(os.path.join(base, it))
        fi = folder_counter.get(parent, 0)
        folder_counter[parent] = fi + 1
        ctx = {
            "index": i, "folder_index": fi, "original": fname, "is_dir": is_dir,
            "folder": os.path.basename(parent) if parent else folder_name,
            "parent": os.path.basename(os.path.dirname(parent)) if parent else parent_name,
            "mtime": st.st_mtime,
            "ctime": getattr(st, "st_birthtime", None) or st.st_ctime,
            "size": st.st_size, "allow_subdirs": allow_subdirs,
            "meta": metas.get(it, {}),
        }
        try:
            new_name = apply_rules(fname, rules, ctx)
        except RuleError as e:
            raise PlanError(str(e))
        err = validate_name(new_name)
        new = _join(parent, new_name)
        plan.append({"old": it, "new": new, "changed": new != it, "dir": is_dir, "error": err})

    if options.get("sidecars"):
        plan += _sidecars(base, plan, options.get("sidecar_exts") or DEFAULT_SIDECARS, set(items))
        # Selected followers whose main file did not change stay as they are
        covered = {r["old"] for r in plan}
        plan += [{"old": f, "new": f, "changed": False, "dir": False, "error": None}
                 for f in followers if f not in covered]

    _check_conflicts(base, plan, conflict_mode)
    return plan


def _sidecar_exts(exts: str) -> set:
    return {e.strip().lstrip(".").lower() for e in exts.split(",") if e.strip()}


def _sidecars(base: str, plan: List[dict], exts: str, selected: set) -> List[dict]:
    """Companion files (movie.srt, photo.xmp) follow their main file's new name."""
    wanted = _sidecar_exts(exts)
    out: List[dict] = []
    listing: Dict[str, List[str]] = {}
    done = {r["old"] for r in plan}
    for row in plan:
        if not row["changed"] or row["dir"] or row["error"]:
            continue
        parent = _parent(row["old"])
        if parent not in listing:
            try:
                listing[parent] = os.listdir(os.path.join(base, parent))
            except OSError:
                listing[parent] = []
        old_stem = split_name(os.path.basename(row["old"]))[0]
        new_rel = row["new"]
        new_stem = split_name(os.path.basename(new_rel))[0]
        new_dir = _parent(new_rel)
        for cand in listing[parent]:
            rel = _join(parent, cand)
            if rel == row["old"] or rel in done or not cand.startswith(old_stem + "."):
                continue
            if split_name(cand)[1].lower() not in wanted:
                continue
            done.add(rel)
            out.append({"old": rel, "new": _join(new_dir, new_stem + cand[len(old_stem):]),
                        "changed": True, "dir": False, "error": None, "sidecar": True,
                        "of": row["old"]})
    return out


def _numbered(path: str, n: int, is_dir: bool) -> str:
    d, name = os.path.dirname(path), os.path.basename(path)
    stem, ext = split_name(name, is_dir)
    return _join(d, f"{stem} ({n})" + (f".{ext}" if ext else ""))


def _check_conflicts(base: str, plan: List[dict], mode: str) -> None:
    moving = {r["old"] for r in plan if r["changed"]}
    taken: Dict[str, dict] = {}
    # Unchanged rows occupy their name
    for r in plan:
        if not r["changed"]:
            taken[r["new"]] = r

    def exists_outside(path: str, row: dict) -> bool:
        full = os.path.join(base, path)
        if not os.path.lexists(full):
            return False
        if path in moving:
            return False
        try:  # case-only rename on a case-insensitive file system
            return not os.path.samefile(full, os.path.join(base, row["old"]))
        except OSError:
            return True

    for r in plan:
        if not r["changed"] or r["error"]:
            continue
        target = r["new"]
        clash = target in taken or exists_outside(target, r)
        if clash and mode == "number":
            n = 2
            while True:
                cand = _numbered(r["new"], n, r["dir"])
                if cand not in taken and not exists_outside(cand, r):
                    target = cand
                    break
                n += 1
            r["new"], r["resolved"] = target, True
            clash = False
        if clash:
            other = taken.get(target)
            r["error"] = t("Doppelter Zielname") if other is not None else t("Datei existiert bereits")
            if other is not None and other["changed"] and not other["error"]:
                other["error"] = "Doppelter Zielname"
        taken[target] = r


# --------------------------------------------------------------------------- #
# Execution
# --------------------------------------------------------------------------- #

def build_phases(plan: List[dict]) -> List[List[Pair]]:
    """Files first, then folders by depth (deepest first)."""
    files = [(r["old"], r["new"]) for r in plan if r["changed"] and not r["dir"]]
    dirs = [r for r in plan if r["changed"] and r["dir"]]
    phases: List[List[Pair]] = [files] if files else []
    by_depth: Dict[int, List[Pair]] = {}
    for r in dirs:
        by_depth.setdefault(r["old"].count("/"), []).append((r["old"], r["new"]))
    for depth in sorted(by_depth, reverse=True):
        phases.append(by_depth[depth])
    return phases


def _move(src: str, dst: str) -> None:
    try:
        os.rename(src, dst)
    except OSError as e:
        if e.errno != errno.EXDEV:
            raise
        shutil.move(src, dst)


def run_phase(base: str, pairs: List[Pair]) -> List[str]:
    """Execute one phase atomically-ish. Returns folders it created."""
    tag = uuid.uuid4().hex[:8]
    j = lambda rel: os.path.join(base, rel)  # noqa: E731
    current = {old: old for old, _ in pairs}
    created: List[str] = []
    try:
        for i, (old, _) in enumerate(pairs):
            tmp = _join(_parent(old), f".rn-{tag}-{i}")
            os.rename(j(old), j(tmp))
            current[old] = tmp
        for old, new in pairs:
            if os.path.lexists(j(new)):
                raise OSError(t("Ziel existiert bereits: {path}", path=new))
            d = _parent(new)
            if d and not os.path.isdir(j(d)):
                os.makedirs(j(d))
                created.append(d)
            _move(j(current[old]), j(new))
            current[old] = new
    except OSError:
        for i, (old, _) in enumerate(pairs):
            if current[old] != old:
                try:
                    back = _join(_parent(old), f".rb-{tag}-{i}")
                    _move(j(current[old]), j(back))
                    current[old] = back
                except OSError:
                    pass
        for old, _ in pairs:
            if current[old] != old and not os.path.lexists(j(old)):
                try:
                    _move(j(current[old]), j(old))
                except OSError:
                    pass
        remove_empty_dirs(base, created)
        raise
    return created


def remove_empty_dirs(base: str, dirs: List[str]) -> None:
    for d in sorted(set(dirs), key=lambda p: p.count("/"), reverse=True):
        try:
            os.rmdir(os.path.join(base, d))
        except OSError:
            pass


def execute(base: str, phases: List[List[Pair]]) -> List[str]:
    """Run all phases; on failure undo the phases already done."""
    done: List[List[Pair]] = []
    created: List[str] = []
    try:
        for ph in phases:
            created += run_phase(base, ph)
            done.append(ph)
    except OSError:
        for ph in reversed(done):
            try:
                run_phase(base, [(new, old) for old, new in ph])
            except OSError:
                pass
        remove_empty_dirs(base, created)
        raise
    return created


def undo(base: str, phases: List[List[Pair]], created: List[str]) -> int:
    if not phases:
        return 0
    # The phase undone first is the last one executed; its targets must still exist.
    # (File paths inside renamed folders only become valid again after that.)
    gone = [n for _, n in phases[-1] if not os.path.lexists(os.path.join(base, n))]
    if gone:
        raise PlanError(t("{n} Element(e) nicht mehr vorhanden, z. B. {example}", n=len(gone), example=gone[0]))
    execute(base, [[(new, old) for old, new in ph] for ph in reversed(phases)])
    remove_empty_dirs(base, created)
    return sum(len(ph) for ph in phases)
