"""Text with positions from simple PDFs – enough for Deutschlandfunk's monthly best-of lists ("Die besten
Sachbücher / Krimis im …"), which exist only as PDF.

Deliberately small and defensive: no external library, only FlateDecode streams are unpacked (with a size
limit each and in total), images are skipped, nothing is executed or rendered. Text is decoded with the font's
ToUnicode map (two-byte Identity-H fonts) or as WinAnsi. Positions come from Tm/Td/TD/T* – enough for column
layouts; rotation and scaling are ignored.
"""
from __future__ import annotations

import re
import zlib
from typing import Dict, List, Optional, Tuple

MAX_PDF = 8 << 20              # bytes of the file
MAX_STREAM = 4 << 20           # unpacked bytes per stream …
MAX_TOTAL = 24 << 20           # … and altogether
MAX_TOKENS = 400_000           # content stream tokens per page

_OBJ = re.compile(rb"(\d{1,7})\s+0\s+obj\b(.*?)\bendobj", re.S)
_TOKEN = re.compile(rb"\((?:\\.|[^\\)])*\)|<[0-9A-Fa-f\s]*>|\[|\]|/[^\s/\[\]()<>{}]+|[-+]?\d*\.?\d+|[A-Za-z'\"*]+", re.S)
_OPERATOR = re.compile(rb"[A-Za-z'\"*]+")

Chunk = Tuple[float, float, str]          # x, y, text


class _Budget:
    def __init__(self) -> None:
        self.left = MAX_TOTAL

    def inflate(self, data: bytes) -> bytes:
        if self.left <= 0:
            return b""
        try:
            out = zlib.decompressobj().decompress(data, min(MAX_STREAM, self.left))
        except zlib.error:
            return b""
        self.left -= len(out)
        return out


def _objects(raw: bytes) -> Dict[int, Tuple[bytes, Optional[bytes]]]:
    """{number: (dictionary, unpacked stream or None)} – object streams (/ObjStm) included."""
    budget = _Budget()
    objs: Dict[int, Tuple[bytes, Optional[bytes]]] = {}
    for m in _OBJ.finditer(raw):
        body = m.group(2)
        sm = re.search(rb"stream\r?\n", body)
        if not sm:
            objs[int(m.group(1))] = (body, None)
            continue
        head, data = body[:sm.start()], body[sm.end():]
        data = data[:data.rfind(b"endstream")]
        if re.search(rb"/Subtype\s*/Image", head):
            data = b""                                            # pictures: not needed
        elif b"/Filter" not in head:
            data = data[:MAX_STREAM]                              # stored as it is
        elif re.search(rb"/Filter\s*(?:/FlateDecode\b|\[\s*/FlateDecode\s*\])", head):
            data = budget.inflate(data)
        else:
            data = b""                                            # other filters: not supported
        objs[int(m.group(1))] = (head, data)
    for head, data in list(objs.values()):
        if not data or not re.search(rb"/Type\s*/ObjStm", head):
            continue
        n, first = re.search(rb"/N\s+(\d+)", head), re.search(rb"/First\s+(\d+)", head)
        if not n or not first:
            continue
        first_at = int(first.group(1))
        nums = [int(x) for x in data[:first_at].split()[:2 * min(int(n.group(1)), 10000)] if x.isdigit()]
        pairs = list(zip(nums[0::2], nums[1::2]))
        for i, (num, off) in enumerate(pairs):
            end = pairs[i + 1][1] if i + 1 < len(pairs) else len(data) - first_at
            objs.setdefault(num, (data[first_at + off:first_at + end], None))
    return objs


def _ref(text: bytes, key: bytes) -> Optional[int]:
    m = re.search(rb"/" + key + rb"\s*(\d+)\s+0\s+R", text)
    return int(m.group(1)) if m else None


def _cmap(data: bytes) -> Dict[int, str]:
    """ToUnicode map: bfchar and bfrange (the plain start-code form)."""
    out: Dict[int, str] = {}
    for block in re.findall(rb"beginbfchar(.*?)endbfchar", data, re.S):
        for a, b in re.findall(rb"<([0-9A-Fa-f]{2,8})>\s*<([0-9A-Fa-f]{2,64})>", block):
            out[int(a, 16)] = bytes.fromhex(b.decode()).decode("utf-16-be", "replace")
    for block in re.findall(rb"beginbfrange(.*?)endbfrange", data, re.S):
        for a, b, c in re.findall(rb"<([0-9A-Fa-f]{2,8})>\s*<([0-9A-Fa-f]{2,8})>\s*<([0-9A-Fa-f]{2,8})>", block):
            lo, hi, start = int(a, 16), int(b, 16), int(c, 16)
            for i in range(lo, min(hi, lo + 4096) + 1):
                if 0 <= start + i - lo < 0x110000:
                    out[i] = chr(start + i - lo)
    return out


def _string(tok: bytes) -> bytes:
    if tok.startswith(b"<"):
        hexed = re.sub(rb"\s", b"", tok[1:-1])
        return bytes.fromhex((hexed + b"0" * (len(hexed) % 2)).decode())
    s, out, i = tok[1:-1], bytearray(), 0
    while i < len(s):
        c = s[i]
        if c == 0x5C and i + 1 < len(s):                         # backslash escape
            n = s[i + 1]
            if n in b"nrtbf":
                out += {0x6E: b"\n", 0x72: b"\r", 0x74: b"\t", 0x62: b"\b", 0x66: b"\f"}[n]
                i += 2
            elif 0x30 <= n <= 0x37:
                j = i + 1
                while j < len(s) and j < i + 4 and 0x30 <= s[j] <= 0x37:
                    j += 1
                out.append(int(s[i + 1:j], 8) & 0xFF)
                i = j
            else:
                out.append(n)
                i += 2
        else:
            out.append(c)
            i += 1
    return bytes(out)


def _fonts(objs, page: bytes) -> Dict[bytes, Tuple[Optional[Dict[int, str]], bool]]:
    res = page
    num = _ref(page, b"Resources")
    if num in objs:
        res = objs[num][0]
    fonts: Dict[bytes, Tuple[Optional[Dict[int, str]], bool]] = {}
    fm = re.search(rb"/Font\s*(<<.*?>>|\d+\s+0\s+R)", res, re.S)
    if not fm:
        return fonts
    table = fm.group(1)
    if table.endswith(b"R"):
        table = objs.get(int(table.split()[0]), (b"", None))[0]
    for name, ref in re.findall(rb"/([^\s/<>\[\]]+)\s+(\d+)\s+0\s+R", table):
        head = objs.get(int(ref), (b"", None))[0]
        tu = _ref(head, b"ToUnicode")
        fonts[name] = (_cmap(objs[tu][1] or b"") if tu in objs else None, b"Identity-H" in head)
    return fonts


def _page_chunks(objs, page: bytes) -> List[Chunk]:
    fonts = _fonts(objs, page)
    cm = re.search(rb"/Contents\s*(\[[^\]]*\]|\d+\s+0\s+R)", page)
    data = b"\n".join(objs.get(int(n), (b"", b""))[1] or b"" for n in re.findall(rb"(\d+)\s+0\s+R", cm.group(1) if cm else b""))
    font: Tuple[Optional[Dict[int, str]], bool] = (None, False)

    def decode(bs: bytes) -> str:
        table, two = font
        if two:
            return "".join((table or {}).get(int.from_bytes(bs[i:i + 2], "big"), "") for i in range(0, len(bs) - 1, 2))
        if table:
            return "".join(table.get(b, bytes([b]).decode("cp1252", "replace")) for b in bs)
        return bs.decode("cp1252", "replace")

    chunks: List[Chunk] = []
    stack: list = []
    x = y = lx = ly = lead = 0.0
    for n, tok in enumerate(_TOKEN.findall(data)):
        if n > MAX_TOKENS:
            break
        try:
            if tok == b"[":
                stack.append(tok)
            elif tok == b"]":
                arr = []
                while stack and stack[-1] != b"[":
                    arr.append(stack.pop())
                if stack:
                    stack.pop()
                stack.append(("array", arr[::-1]))
            elif _OPERATOR.fullmatch(tok):
                if tok == b"Tf" and len(stack) >= 2 and isinstance(stack[-2], bytes):
                    font = fonts.get(stack[-2][1:], (None, False))
                elif tok == b"Tm" and len(stack) >= 6:
                    x = lx = float(stack[-2])
                    y = ly = float(stack[-1])
                elif tok in (b"Td", b"TD") and len(stack) >= 2:
                    lx += float(stack[-2])
                    ly += float(stack[-1])
                    x, y = lx, ly
                    if tok == b"TD":
                        lead = -float(stack[-1])
                elif tok == b"TL" and stack:
                    lead = float(stack[-1])
                elif tok == b"T*":
                    ly -= lead
                    x, y = lx, ly
                elif tok == b"BT":
                    x = y = lx = ly = 0.0
                elif tok in (b"Tj", b"'", b'"') and stack and isinstance(stack[-1], bytes):
                    chunks.append((x, y, decode(_string(stack[-1]))))
                elif tok == b"TJ" and stack and isinstance(stack[-1], tuple):
                    text = ""
                    for e in stack[-1][1]:
                        if isinstance(e, bytes) and e[:1] in (b"(", b"<"):
                            text += decode(_string(e))
                        elif isinstance(e, bytes) and float(e) < -200:
                            text += " "
                    chunks.append((x, y, text))
                stack = []
            else:
                stack.append(tok)
        except (ValueError, IndexError):
            stack = []
    return chunks


def pages(raw: bytes) -> List[List[Chunk]]:
    """Every page's text pieces with their positions (x to the right, y upwards)."""
    if not raw.startswith(b"%PDF") or len(raw) > MAX_PDF:
        raise ValueError("no PDF")
    objs = _objects(raw)
    return [_page_chunks(objs, head) for _, (head, _) in sorted(objs.items())
            if re.search(rb"/Type\s*/Page\b", head)]


_RANK = re.compile(r"^\s*(\d{1,2})\s*\(\s*(?:\d{1,2}|-|–|neu)\s*\)\s*$", re.I)
_STOP = re.compile(r"^(?:Übersetzt|Aus dem|Herausgegeben|Hrsg|Mit |Illustriert|Gelesen|\d+\s+Seiten|\d+[,.]?\d*\s+Euro)", re.I)


def best_list(raw: bytes) -> List[dict]:
    """Deutschlandfunk's best-of lists: [{rank, title, author}] – each entry is a column starting with
    "3 (5)" (place, last month's place), then the title, then the author(s) indented beneath it."""
    out: List[dict] = []
    for chunks in pages(raw):
        lines: Dict[int, List[Tuple[float, str]]] = {}
        for x, y, s in chunks:
            lines.setdefault(round(y), []).append((x, s))
        anchors = []
        for y, cs in lines.items():
            groups: List[list] = []
            for x, s in sorted(cs):
                if groups and x - groups[-1][1] < 25:
                    groups[-1][1] = x
                    groups[-1][2].append(s)
                else:
                    groups.append([x, x, [s]])
            for gx, _, parts in groups:
                m = _RANK.match("".join(parts))
                if m:
                    anchors.append((gx, y, int(m.group(1))))
        for ax, ay, rank in anchors:
            right = min([x for x, y, _ in anchors if y == ay and x > ax] + [ax + 200])
            column: Dict[int, List[Tuple[float, str]]] = {}
            for x, y, s in chunks:
                if ax - 3 <= x < right - 3 and ay - 260 < y < ay - 1:
                    column.setdefault(round(y), []).append((x, s))
            title: List[str] = []
            author: List[str] = []
            for y in sorted(column, reverse=True):
                cs = sorted(column[y])
                text = "".join(s for _, s in cs).strip()
                if not text:
                    continue
                indented = cs[0][0] >= ax + 40
                if not indented and not author:
                    if _RANK.match(text):
                        break
                    title.append(text)
                elif indented and title:
                    if _STOP.match(text):
                        break
                    author.append(text)
                elif author:
                    break
            if title and author:
                out.append({"rank": rank, "title": re.sub(r"\s+", " ", " ".join(title)).strip(),
                            "author": re.sub(r"\s+", " ", " ".join(author)).strip()})
    return sorted(out, key=lambda e: e["rank"])
