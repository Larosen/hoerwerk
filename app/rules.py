"""Rename rule engine (feature set modelled on A Better Finder Rename).

Every rule receives the current name split into (stem, ext) plus a context
dict and returns a new (stem, ext). Rules are applied in order.

Context keys (filled by the planner):
    index         position in the whole selection (0-based)
    folder_index  position among the selected items of the same folder
    original      original file name
    folder        name of the containing folder, ``parent`` the one above
    mtime/ctime   timestamps, ``size`` in bytes, ``is_dir``
    meta          lazily loaded metadata (ID3/video/EXIF), see meta.py
"""
from __future__ import annotations

import os
import re
import unicodedata
from datetime import datetime
from typing import Callable, Dict, List, Optional, Tuple
from i18n import t

Name = Tuple[str, str]  # (stem, ext without dot)


class RuleError(Exception):
    pass


def split_name(filename: str, is_dir: bool = False) -> Name:
    if is_dir:
        return filename, ""
    stem, ext = os.path.splitext(filename)
    if not stem:  # dotfiles like ".env"
        return filename, ""
    return stem, ext[1:] if ext else ""


def join_name(stem: str, ext: str) -> str:
    return f"{stem}.{ext}" if ext else stem


def _target_get(name: Name, target: str) -> str:
    stem, ext = name
    if target == "ext":
        return ext
    if target == "all":
        return join_name(stem, ext)
    return stem


def _target_set(name: Name, target: str, value: str) -> Name:
    stem, ext = name
    if target == "ext":
        return stem, value
    if target == "all":
        return split_name(value)
    return value, ext


def _int(v, default=0) -> int:
    try:
        return int(v)
    except (TypeError, ValueError):
        return default


def _compile(pattern: str, regex: bool, case_sensitive: bool, whole_word: bool = False):
    flags = 0 if case_sensitive else re.IGNORECASE
    src = pattern if regex else re.escape(pattern)
    if whole_word:
        src = rf"(?<!\w)(?:{src})(?!\w)"
    try:
        return re.compile(src, flags)
    except re.error as e:
        raise RuleError(t("Ungültiger regulärer Ausdruck „{pattern}“: {error}", pattern=pattern, error=e))


def _expand_repl(repl: str, regex: bool):
    """Regex replacements may use \\1 or $1; literal ones are taken as-is."""
    if not regex:
        return lambda m: repl
    repl = re.sub(r"\$(\d+)", r"\\g<\1>", repl)
    return lambda m: m.expand(repl)


# --------------------------------------------------------------------------- #
# Text rules
# --------------------------------------------------------------------------- #

def rule_replace(name: Name, r: dict, ctx: dict) -> Name:
    target = r.get("target", "stem")
    text = _target_get(name, target)
    find = r.get("find", "")
    if not find:
        return name
    regex = bool(r.get("regex"))
    rx = _compile(find, regex, bool(r.get("case_sensitive")), bool(r.get("whole_word")))
    repl = _expand_repl(r.get("replace", ""), regex)
    occurrence = r.get("occurrence") or ("first" if r.get("first_only") else "all")
    try:
        if occurrence == "first":
            text = rx.sub(repl, text, count=1)
        elif occurrence == "last":
            matches = list(rx.finditer(text))
            if matches:
                m = matches[-1]
                text = text[:m.start()] + repl(m) + text[m.end():]
        else:
            text = rx.sub(repl, text)
    except (re.error, IndexError) as e:
        raise RuleError(t("Ersetzung fehlerhaft: {error}", error=e))
    return _target_set(name, target, text)


SMALL_WORDS = {
    # English
    "a", "an", "and", "as", "at", "but", "by", "for", "in", "nor", "of", "on", "or",
    "the", "to", "vs", "via", "with",
    # German
    "und", "oder", "der", "die", "das", "des", "dem", "den", "ein", "eine", "einer",
    "eines", "einem", "einen", "im", "am", "an", "auf", "aus", "bei", "mit", "nach",
    "von", "vom", "zu", "zum", "zur", "für", "über", "unter", "vor", "um",
}

_WORD = re.compile(r"[^\W_]+(?:['’][^\W_]+)*", re.UNICODE)


def _title_case(s: str, small_words: bool) -> str:
    words = list(_WORD.finditer(s))
    out, last = [], 0
    for i, m in enumerate(words):
        w = m.group(0)
        out.append(s[last:m.start()])
        lower = w.lower()
        edge = i == 0 or i == len(words) - 1
        prev = s[words[i - 1].end():m.start()] if i else ""
        after_break = any(c in prev for c in "-–(:[.")
        if small_words and lower in SMALL_WORDS and not edge and not after_break:
            out.append(lower)
        else:
            out.append(lower[:1].upper() + lower[1:])
        last = m.end()
    out.append(s[last:])
    return "".join(out)


def rule_case(name: Name, r: dict, ctx: dict) -> Name:
    target = r.get("target", "stem")
    text = _target_get(name, target)
    mode = r.get("mode", "title")
    if mode == "upper":
        text = text.upper()
    elif mode == "lower":
        text = text.lower()
    elif mode == "title":
        text = _title_case(text, bool(r.get("small_words", True)))
    elif mode == "words":
        text = _title_case(text, False)
    elif mode == "sentence":
        text = text.lower()
        text = re.sub(r"(^|[.!?]\s+|\s-\s)(\w)", lambda m: m.group(1) + m.group(2).upper(), text)
    elif mode == "swap":
        text = text.swapcase()
    # Words that must keep their exact spelling (e.g. "MP3, DJ, McCartney")
    for word in [w.strip() for w in str(r.get("keep", "")).split(",") if w.strip()]:
        text = re.sub(rf"(?<!\w){re.escape(word)}(?!\w)", word, text, flags=re.IGNORECASE)
    return _target_set(name, target, text)


def rule_insert(name: Name, r: dict, ctx: dict) -> Name:
    target = r.get("target", "stem")
    stem = _target_get(name, target)
    text = expand_template(r.get("text", ""), name, ctx)
    pos = r.get("position", "start")
    if pos == "start":
        stem = text + stem
    elif pos == "end":
        stem = stem + text
    elif pos in ("before", "after"):
        anchor = r.get("anchor", "")
        if not anchor:
            return name
        rx = _compile(anchor, bool(r.get("regex")), bool(r.get("case_sensitive")))
        m = rx.search(stem)
        if not m:
            return name
        at = m.start() if pos == "before" else m.end()
        stem = stem[:at] + text + stem[at:]
    else:  # index
        idx = max(_int(r.get("index"), 0), 0)
        if r.get("from_end"):
            idx = max(len(stem) - idx, 0)
        stem = stem[:idx] + text + stem[idx:]
    return _target_set(name, target, stem)


def rule_remove(name: Name, r: dict, ctx: dict) -> Name:
    """Remove a range of characters, or everything before/after a marker."""
    stem, ext = name
    mode = r.get("mode", "range")
    if mode in ("before", "after"):
        anchor = r.get("anchor", "")
        if not anchor:
            return name
        rx = _compile(anchor, bool(r.get("regex")), bool(r.get("case_sensitive")))
        matches = list(rx.finditer(stem))
        if not matches:
            return name
        m = matches[-1] if r.get("last") else matches[0]
        keep_marker = bool(r.get("keep_anchor"))
        if mode == "before":
            stem = stem[m.start() if keep_marker else m.end():]
        else:
            stem = stem[:m.end() if keep_marker else m.start()]
        return stem, ext
    start = max(_int(r.get("start"), 0), 0)
    count = max(_int(r.get("count"), 0), 0)
    if count == 0:
        return name
    if r.get("from_end"):
        end = max(len(stem) - start, 0)
        stem = stem[:max(end - count, 0)] + stem[end:]
    else:
        stem = stem[:start] + stem[start + count:]
    return stem, ext


_CHAR_CLASSES = {
    "digits": r"\d",
    "letters": r"[^\W\d_]",
    "spaces": r"\s",
    "punctuation": r"[^\w\s]|_",
    "non_alnum": r"[^\w]|_",
    "non_ascii": r"[^\x00-\x7f]",
}


def rule_remove_chars(name: Name, r: dict, ctx: dict) -> Name:
    target = r.get("target", "stem")
    text = _target_get(name, target)
    for key, pattern in _CHAR_CLASSES.items():
        if r.get(key):
            text = re.sub(pattern, "", text)
    custom = r.get("chars", "")
    if custom:
        text = "".join(c for c in text if c not in custom)
    words = [w.strip() for w in str(r.get("words", "")).split(",") if w.strip()]
    for w in words:
        text = re.sub(rf"(?<!\w){re.escape(w)}(?!\w)", "", text, flags=re.IGNORECASE)
    if words:
        text = re.sub(r"\s{2,}", " ", text).strip()
    return _target_set(name, target, text)


WINDOWS_BAD = re.compile(r'[<>:"/\\|?*\x00-\x1f]')


def rule_cleanup(name: Name, r: dict, ctx: dict) -> Name:
    stem, ext = name
    if r.get("underscores", True):
        stem = stem.replace("_", " ")
    if r.get("dots"):
        stem = stem.replace(".", " ")
    if r.get("percent"):
        stem = stem.replace("%20", " ").replace("+", " ")
    if r.get("dashes"):
        stem = re.sub(r"\s*[-–—]\s*", " - ", stem)
    if r.get("strip_brackets"):
        stem = re.sub(r"\s*[\(\[\{][^\)\]\}]*[\)\]\}]", "", stem)
    if r.get("strip_digits_start"):
        stem = re.sub(r"^[\d\s.\-_]+", "", stem)
    if r.get("ascii"):
        stem = transliterate(stem)
    if r.get("windows"):
        stem = WINDOWS_BAD.sub(r.get("windows_char", "-") or "", stem)
        ext = WINDOWS_BAD.sub("", ext)
    if r.get("collapse", True):
        stem = re.sub(r"\s{2,}", " ", stem)
    if r.get("trim", True):
        stem = stem.strip(" .-_")
    return stem, ext


_TRANSLIT = str.maketrans({"ä": "ae", "ö": "oe", "ü": "ue", "Ä": "Ae", "Ö": "Oe", "Ü": "Ue",
                           "ß": "ss", "æ": "ae", "Æ": "Ae", "ø": "o", "Ø": "O", "œ": "oe",
                           "Œ": "Oe", "đ": "d", "Đ": "D", "ł": "l", "Ł": "L", "þ": "th"})


def transliterate(s: str) -> str:
    s = s.translate(_TRANSLIT)
    return unicodedata.normalize("NFKD", s).encode("ascii", "ignore").decode()


def rule_truncate(name: Name, r: dict, ctx: dict) -> Name:
    stem, ext = name
    n = max(_int(r.get("length"), 0), 1)
    if len(stem) <= n:
        return name
    if r.get("keep") == "end":
        stem = stem[-n:]
    else:
        stem = stem[:n]
        if r.get("word_boundary") and " " in stem:
            stem = stem.rsplit(" ", 1)[0]
    return stem.strip(), ext


# --------------------------------------------------------------------------- #
# Position rules
# --------------------------------------------------------------------------- #

def rule_move(name: Name, r: dict, ctx: dict) -> Name:
    """Move matching text to the start or end of the name."""
    stem, ext = name
    find = r.get("find", "")
    if not find:
        return name
    rx = _compile(find, bool(r.get("regex")), bool(r.get("case_sensitive")))
    m = rx.search(stem)
    if not m:
        return name
    piece = m.group(0)
    rest = (stem[:m.start()] + stem[m.end():]).strip(" -_.")
    rest = re.sub(r"\s{2,}", " ", rest)
    sep = r.get("separator", " ")
    stem = f"{piece}{sep}{rest}" if r.get("to", "start") == "start" else f"{rest}{sep}{piece}"
    return stem, ext


def rule_swap(name: Name, r: dict, ctx: dict) -> Name:
    """'Artist - Title' -> 'Title - Artist'."""
    stem, ext = name
    sep = r.get("separator", " - ")
    if not sep or sep not in stem:
        return name
    if r.get("occurrence") == "last":
        a, b = stem.rsplit(sep, 1)
    else:
        a, b = stem.split(sep, 1)
    joiner = r.get("joiner") or sep
    return f"{b}{joiner}{a}", ext


# --------------------------------------------------------------------------- #
# Numbers
# --------------------------------------------------------------------------- #

def _roman(n: int) -> str:
    if n <= 0:
        return str(n)
    vals = [(1000, "M"), (900, "CM"), (500, "D"), (400, "CD"), (100, "C"), (90, "XC"),
            (50, "L"), (40, "XL"), (10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I")]
    out = ""
    for v, s in vals:
        while n >= v:
            out += s
            n -= v
    return out


def _alpha(n: int) -> str:
    if n <= 0:
        return str(n)
    out = ""
    while n:
        n, rem = divmod(n - 1, 26)
        out = chr(97 + rem) + out
    return out


def format_number(n: int, fmt: str, pad: int) -> str:
    if fmt == "alpha":
        return _alpha(n)
    if fmt == "ALPHA":
        return _alpha(n).upper()
    if fmt == "roman":
        return _roman(n)
    if fmt == "roman_lower":
        return _roman(n).lower()
    s = str(abs(n)).zfill(pad)
    return "-" + s if n < 0 else s


def rule_number(name: Name, r: dict, ctx: dict) -> Name:
    stem, ext = name
    start = _int(r.get("start"), 1)
    step = _int(r.get("step"), 1)
    pad = max(_int(r.get("padding"), 2), 0)
    idx = ctx.get("folder_index", ctx["index"]) if r.get("per_folder") else ctx["index"]
    num = format_number(start + idx * step, r.get("format", "decimal"), pad)
    num = r.get("prefix", "") + num + r.get("suffix", "")
    sep = r.get("separator", " - ")
    pos = r.get("position", "start")
    if pos == "start":
        stem = f"{num}{sep}{stem}"
    elif pos == "end":
        stem = f"{stem}{sep}{num}"
    elif pos == "index":
        i = max(_int(r.get("index"), 0), 0)
        stem = stem[:i] + num + stem[i:]
    else:  # replace whole name
        stem = num
    return stem, ext


_NUM = re.compile(r"\d+")


def _number_matches(stem: str, which: str):
    ms = list(_NUM.finditer(stem))
    if which == "first":
        return ms[:1]
    if which == "last":
        return ms[-1:]
    return ms


def rule_pad_numbers(name: Name, r: dict, ctx: dict) -> Name:
    stem, ext = name
    width = max(_int(r.get("width"), 2), 1)
    strip = bool(r.get("strip_zeros"))
    for m in reversed(_number_matches(stem, r.get("which", "all"))):
        digits = m.group(0).lstrip("0") or "0" if strip else m.group(0)
        stem = stem[:m.start()] + digits.zfill(width) + stem[m.end():]
    return stem, ext


def rule_shift_numbers(name: Name, r: dict, ctx: dict) -> Name:
    stem, ext = name
    offset = _int(r.get("offset"), 1)
    for m in reversed(_number_matches(stem, r.get("which", "first"))):
        old = m.group(0)
        new = max(int(old) + offset, 0)
        stem = stem[:m.start()] + str(new).zfill(len(old)) + stem[m.end():]
    return stem, ext


# --------------------------------------------------------------------------- #
# Dates, lists, templates, extension
# --------------------------------------------------------------------------- #

DATE_SOURCES = ("modified", "created", "taken", "now")


def get_date(source: str, ctx: dict) -> Optional[datetime]:
    if source == "now":
        return datetime.now()
    if source == "created":
        return datetime.fromtimestamp(ctx.get("ctime") or ctx.get("mtime") or 0)
    if source == "taken":
        meta = _meta(ctx)
        return meta.get("_taken")
    return datetime.fromtimestamp(ctx.get("mtime") or 0)


def rule_date(name: Name, r: dict, ctx: dict) -> Name:
    stem, ext = name
    d = get_date(r.get("source", "modified"), ctx)
    if d is None:
        if r.get("fallback", True):
            d = get_date("modified", ctx)
        else:
            return name
    fmt = r.get("format") or "%Y-%m-%d"
    try:
        text = d.strftime(fmt)
    except ValueError as e:
        raise RuleError(t("Ungültiges Datumsformat: {error}", error=e))
    sep = r.get("separator", " ")
    pos = r.get("position", "start")
    if pos == "start":
        stem = f"{text}{sep}{stem}"
    elif pos == "end":
        stem = f"{stem}{sep}{text}"
    else:
        stem = text
    return stem, ext


def rule_list(name: Name, r: dict, ctx: dict) -> Name:
    """Rename from a pasted list: line i is used for the i-th file."""
    stem, ext = name
    lines = [ln.strip() for ln in str(r.get("lines", "")).splitlines()]
    if not r.get("keep_empty"):
        lines = [ln for ln in lines if ln]
    i = ctx["index"]
    if i >= len(lines):
        return name
    line = lines[i]
    mode = r.get("mode", "replace")
    sep = r.get("separator", " - ")
    if mode == "prefix":
        stem = f"{line}{sep}{stem}"
    elif mode == "suffix":
        stem = f"{stem}{sep}{line}"
    elif mode == "with_ext":
        return split_name(line)
    else:
        stem = line
    return stem, ext


def rule_template(name: Name, r: dict, ctx: dict) -> Name:
    stem, ext = name
    tpl = r.get("template", "{name}")
    # Files without the referenced tags (e.g. a .txt next to the MP3s) stay unchanged
    keys = [m.group(1).lower() for m in _TPL.finditer(tpl)]
    tag_keys = [k for k in keys if k not in BUILTIN_KEYS]
    if tag_keys and not any(_lookup(k, name, ctx) for k in tag_keys):
        return name
    return expand_template(tpl, name, ctx), ext


EXT_ALIASES = {"jpeg": "jpg", "tiff": "tif", "mpeg": "mpg", "htm": "html", "yml": "yaml"}


def rule_extension(name: Name, r: dict, ctx: dict) -> Name:
    stem, ext = name
    if ctx.get("is_dir"):
        return name
    mode = r.get("mode", "lower")
    if mode == "lower":
        ext = ext.lower()
    elif mode == "upper":
        ext = ext.upper()
    elif mode == "set":
        ext = r.get("value", "").lstrip(".")
    elif mode == "append":
        ext = (ext + "." if ext else "") + r.get("value", "").lstrip(".")
    elif mode == "normalize":
        ext = EXT_ALIASES.get(ext.lower(), ext.lower())
    elif mode == "remove":
        ext = ""
    return stem, ext


RULES: Dict[str, Callable[[Name, dict, dict], Name]] = {
    "replace": rule_replace,
    "case": rule_case,
    "insert": rule_insert,
    "remove": rule_remove,
    "remove_chars": rule_remove_chars,
    "cleanup": rule_cleanup,
    "truncate": rule_truncate,
    "move": rule_move,
    "swap": rule_swap,
    "number": rule_number,
    "pad_numbers": rule_pad_numbers,
    "shift_numbers": rule_shift_numbers,
    "date": rule_date,
    "list": rule_list,
    "template": rule_template,
    "extension": rule_extension,
}


# --------------------------------------------------------------------------- #
# Templates
# --------------------------------------------------------------------------- #

_TPL = re.compile(r"\{(\w+)(?::([^{}]+))?\}")
BUILTIN_KEYS = {"name", "original", "ext", "n", "counter", "folder", "parent", "date",
                "modified", "created", "now", "size", "bytes", "folder_n"}
DATE_KEYS = {"date": "modified", "modified": "modified", "created": "created",
             "now": "now", "taken": "taken"}


def _meta(ctx: dict) -> dict:
    loader = ctx.get("meta_loader")
    if "meta" not in ctx:
        ctx["meta"] = loader() if loader else {}
    return ctx["meta"] or {}


def _human_size(n: int) -> str:
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if n < 1024 or unit == "TB":
            return f"{n:.0f} {unit}" if unit == "B" else f"{n:.1f} {unit}"
        n /= 1024
    return str(n)


def _lookup(key: str, name: Name, ctx: dict):
    stem, ext = name
    if key == "name":
        return stem
    if key == "original":
        return split_name(ctx["original"], ctx.get("is_dir", False))[0]
    if key == "ext":
        return ext
    if key in ("n", "counter"):
        return ctx["index"] + 1
    if key == "folder_n":
        return ctx.get("folder_index", ctx["index"]) + 1
    if key == "folder":
        return ctx.get("folder", "")
    if key == "parent":
        return ctx.get("parent", "")
    if key == "bytes":
        return ctx.get("size", 0)
    if key == "size":
        return _human_size(ctx.get("size", 0))
    if key in DATE_KEYS:
        return get_date(DATE_KEYS[key], ctx)
    meta = _meta(ctx)
    if key in ("track", "disc"):
        return re.split(r"[/\s]", str(meta.get(key, "")))[0]
    if key == "year":
        y = meta.get("year") or meta.get("date") or ""
        m = re.search(r"\d{4}", str(y))
        return m.group(0) if m else ""
    return meta.get(key, "")


def _format(val, fmt: Optional[str]) -> str:
    if isinstance(val, datetime):
        try:
            return val.strftime(fmt or "%Y-%m-%d")
        except ValueError:
            return val.strftime("%Y-%m-%d")
    if val is None:
        return ""
    s = str(val)
    if not fmt:
        return s
    if fmt.isdigit():  # {n:3} / {track:02}
        return s.zfill(int(fmt)) if s.lstrip("-").isdigit() else s
    f = fmt.lower()
    if f == "upper":
        return s.upper()
    if f == "lower":
        return s.lower()
    if f == "title":
        return _title_case(s, True)
    if f == "ascii":
        return transliterate(s)
    m = re.fullmatch(r"(\d*)-(\d*)", fmt)  # {name:0-10} substring
    if m:
        a = int(m.group(1) or 0)
        b = int(m.group(2)) if m.group(2) else None
        return s[a:b]
    return s


def expand_template(tpl: str, name: Name, ctx: dict) -> str:
    return _TPL.sub(lambda m: _format(_lookup(m.group(1).lower(), name, ctx), m.group(2)), tpl)


# --------------------------------------------------------------------------- #
# Applying rules
# --------------------------------------------------------------------------- #

def _rule_applies(r: dict, current: str) -> bool:
    cond = (r.get("only") or "").strip()
    if not cond:
        return True
    mode = r.get("only_mode", "contains")
    if mode == "regex":
        try:
            hit = re.search(cond, current, re.IGNORECASE) is not None
        except re.error as e:
            raise RuleError(t("Ungültige Bedingung „{condition}“: {error}", condition=cond, error=e))
    elif mode == "ext":
        exts = {e.strip().lstrip(".").lower() for e in cond.split(",") if e.strip()}
        hit = split_name(current)[1].lower() in exts
    else:
        hit = cond.lower() in current.lower()
    return hit != bool(r.get("only_invert"))


INVALID = re.compile(r"[\x00]")


def apply_rules(filename: str, rules: List[dict], ctx: dict) -> str:
    is_dir = bool(ctx.get("is_dir"))
    name = split_name(filename, is_dir)
    for r in rules:
        if not r.get("enabled", True):
            continue
        fn = RULES.get(r.get("type", ""))
        if not fn:
            continue
        if not _rule_applies(r, join_name(*name)):
            continue
        if is_dir and r.get("type") == "extension":
            continue
        name = fn(name, r, ctx)
    result = join_name(*name)
    if not ctx.get("allow_subdirs"):
        result = result.replace("/", "-")
    return INVALID.sub("", result)


def needs_meta(rules: List[dict]) -> bool:
    for r in rules:
        if not r.get("enabled", True):
            continue
        if r.get("type") == "date" and r.get("source") == "taken":
            return True
        if r.get("type") in ("template", "insert"):
            text = (r.get("template") or "") + (r.get("text") or "")
            for m in _TPL.finditer(text):
                if m.group(1).lower() not in BUILTIN_KEYS:
                    return True
    return False


def natural_key(s: str):
    return [int(t) if t.isdigit() else t.lower() for t in re.split(r"(\d+)", s)]


def validate_name(new: str) -> Optional[str]:
    parts = new.split("/")
    for p in parts:
        if not p or p in (".", ".."):
            return t("Leerer oder ungültiger Name")
        if len(p.encode("utf-8")) > 255:
            return t("Name zu lang (max. 255 Byte)")
    if new.startswith("/") or ".." in parts:
        return t("Ungültiger Pfad")
    return None
