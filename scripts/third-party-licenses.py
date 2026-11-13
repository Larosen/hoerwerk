#!/usr/bin/env python3
"""Writes THIRD-PARTY-LICENSES.md (licenses of all Rust libraries in the build): python3 scripts/third-party-licenses.py > THIRD-PARTY-LICENSES.md"""
import json,os,re,subprocess,sys
root=os.path.join(os.path.dirname(os.path.abspath(__file__)),"..","rust")
m=json.loads(subprocess.check_output(["cargo","metadata","--format-version","1","--locked","--filter-platform","x86_64-unknown-linux-gnu"],cwd=root))
pk={p["id"]:p for p in m["packages"]}
nodes={n["id"]:n for n in m["resolve"]["nodes"]}
ws=set(m["workspace_members"])
seen=set(); st=list(ws)
while st:
    i=st.pop()
    if i in seen: continue
    seen.add(i)
    for d in nodes[i]["deps"]:
        if any(k["kind"] in (None,"build") for k in d["dep_kinds"]): st.append(d["pkg"])
out=[]
for i in sorted(seen-ws,key=lambda i:(pk[i]["name"],pk[i]["version"])):
    p=pk[i]; d=os.path.dirname(p["manifest_path"])
    files=sorted(f for f in os.listdir(d) if re.match(r"(?i)(licen[sc]e|copying|notice|unlicense)",f) and os.path.isfile(os.path.join(d,f)))
    out.append((p["name"],p["version"],p.get("license") or p.get("license_file") or "see source",p.get("repository") or "",[(f,open(os.path.join(d,f),errors="replace").read().strip()) for f in files]))
w=sys.stdout.write
w("# Third-party licenses\n\nHörwerk itself is under the [PolyForm Noncommercial License 1.0.0](LICENSE). It is built from the Rust libraries\nbelow, each under its own license (generated from `rust/Cargo.lock`; regenerate with\n`scripts/third-party-licenses.py`). FFmpeg in the Docker image is LGPL 2.1, see [README](README.md#license).\n\n## Overview\n\n| Crate | Version | License |\n|---|---|---|\n")
for n,v,l,r,f in out: w(f"| {n} | {v} | {l} |\n")
w("\n## Texts\n")
for n,v,l,r,f in out:
    w(f"\n### {n} {v} – {l}\n\n")
    if r: w(f"{r}\n\n")
    if not f: w("_No license file in the package; see the license named above._\n")
    for fn,t in f: w(f"`{fn}`\n\n```text\n{t}\n```\n\n")
