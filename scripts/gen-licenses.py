#!/usr/bin/env python3
"""Collects the license notices of everything compiled into PrepareAudio.

Lists every crate linked into the macOS binary (normal dependencies for the
target, no build tools), reads its license files from the cargo registry and
writes
  ui/licenses.json           shown in the app's info panel
  THIRD_PARTY_LICENSES.md    shipped inside the app bundle and in the repository

Crates that ship no license file get the standard text of their SPDX license,
taken from another crate under the same license. LAME 3.100, which mp3lame-sys
compiles into the app, is listed with its own COPYING file.

usage: python3 scripts/gen-licenses.py [--target aarch64-apple-darwin]
"""
import argparse
import hashlib
import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TAURI = os.path.join(ROOT, "src-tauri")
LICENSE_FILE = re.compile(r"^(licen[cs]e|copying|notice|unlicense|copyright)([-._].*)?$", re.I)
MIT_TEMPLATE = """Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE."""


BSD3_TEMPLATE = """Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its
   contributors may be used to endorse or promote products derived from
   this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE."""


def cargo(*args):
    env = dict(os.environ)
    env["PATH"] = os.path.expanduser("~/.cargo/bin") + os.pathsep + env.get("PATH", "")
    return subprocess.run(["cargo", *args], cwd=TAURI, env=env, check=True, capture_output=True, text=True).stdout


def license_files(directory, depth=0):
    found = []
    try:
        entries = sorted(os.listdir(directory))
    except OSError:
        return found
    for name in entries:
        path = os.path.join(directory, name)
        if os.path.isfile(path) and LICENSE_FILE.match(name) and os.path.getsize(path) < 300_000:
            found.append(path)
    return found


def read(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.read().strip()


def spdx_ids(expr):
    return sorted(set(re.findall(r"[A-Za-z0-9.\-+]+", (expr or "").replace("/", " OR "))) - {"OR", "AND", "WITH"})


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", default="aarch64-apple-darwin")
    a = ap.parse_args()

    meta = json.loads(cargo("metadata", "--format-version", "1"))
    tree = cargo("tree", "-e", "normal", "--target", a.target, "--prefix", "none", "-f", "{p}")
    wanted = set()
    for line in tree.splitlines():
        m = re.match(r"^(\S+) v(\S+)", line.strip())
        if m:
            wanted.add((m.group(1), m.group(2)))
    root = next(p for p in meta["packages"] if os.path.dirname(p["manifest_path"]) == TAURI)
    packages = [p for p in meta["packages"] if (p["name"], p["version"]) in wanted and p["id"] != root["id"]]
    packages.sort(key=lambda p: (p["name"].lower(), p["version"]))

    texts = {}  # hash -> {"text", "crates"}
    def add_text(text, crate):
        h = hashlib.sha1(text.encode()).hexdigest()[:12]
        entry = texts.setdefault(h, {"id": h, "text": text, "crates": []})
        if crate not in entry["crates"]:
            entry["crates"].append(crate)
        return h

    # Canonical texts per SPDX id, from crates that ship exactly one license.
    canonical = {}
    for p in packages:
        ids = spdx_ids(p.get("license"))
        files = license_files(os.path.dirname(p["manifest_path"]))
        for f in files:
            body = read(f)
            base = os.path.basename(f).upper()
            if "APACHE" in base and "Apache License" in body and "Version 2.0" in body:
                canonical.setdefault("Apache-2.0", body)
            elif "Mozilla Public License Version 2.0" in body:
                canonical.setdefault("MPL-2.0", body)
            elif "UNICODE" in base or "UNICODE LICENSE V3" in body.upper():
                canonical.setdefault("Unicode-3.0", body)
            elif base.startswith("LICENSE-ZLIB") or (ids == ["Zlib"] and "zlib" in body.lower()):
                canonical.setdefault("Zlib", body)

    crates = []
    for p in packages:
        crate = f'{p["name"]} {p["version"]}'
        directory = os.path.dirname(p["manifest_path"])
        files = license_files(directory)
        note = None
        ids = []
        if files:
            for f in files:
                ids.append(add_text(read(f), crate))
        else:
            for spdx in spdx_ids(p.get("license")):
                if spdx in ("MIT", "MIT-0"):
                    authors = ", ".join(p.get("authors") or []) or f'the {p["name"]} authors'
                    ids.append(add_text(f"Copyright (c) {authors}\n\n{MIT_TEMPLATE}", crate))
                elif spdx == "BSD-3-Clause":
                    authors = ", ".join(p.get("authors") or []) or f'the {p["name"]} authors'
                    ids.append(add_text(f"Copyright (c) {authors}\n\n{BSD3_TEMPLATE}", crate))
                elif spdx in canonical:
                    ids.append(add_text(canonical[spdx], crate))
            note = "Das Paket enthält keine eigene Lizenzdatei; aufgeführt ist der Standardtext der angegebenen Lizenz."
        if p["name"] == "mp3lame-sys":
            lame = next((os.path.join(directory, d) for d in os.listdir(directory) if d.startswith("lame-")), None)
            if lame:
                for f in license_files(lame):
                    ids.append(add_text(read(f), crate + " (LAME 3.100)"))
                note = ("Enthält die Quellen von LAME 3.100 (GNU LGPL), die beim Bauen statisch in die App übersetzt werden. "
                        "Quellen: https://lame.sourceforge.io und das Paket selbst; die App lässt sich aus ihrem Quelltext "
                        "mit einer geänderten LAME-Version neu bauen.")
        if p["name"].startswith("symphonia"):
            note = (note + " " if note else "") + "Quellcode (MPL-2.0): https://github.com/pdeljanov/Symphonia"
        crates.append({
            "name": p["name"],
            "version": p["version"],
            "license": p.get("license") or ("siehe Lizenzdatei" if files else "unbekannt"),
            "repository": p.get("repository") or p.get("homepage") or f'https://crates.io/crates/{p["name"]}',
            "authors": p.get("authors") or [],
            "texts": ids,
            "note": note,
        })

    app_license = read(os.path.join(ROOT, "LICENSE"))
    out = {
        "app": {"name": "PrepareAudio", "version": root["version"], "license": root.get("license") or "MIT",
                "authors": root.get("authors") or [], "license_text": app_license},
        "generated_for": a.target,
        "crates": crates,
        "texts": sorted(texts.values(), key=lambda t: (-len(t["crates"]), t["id"])),
    }
    with open(os.path.join(ROOT, "ui", "licenses.json"), "w", encoding="utf-8") as f:
        json.dump(out, f, ensure_ascii=False, separators=(",", ":"))

    lines = [
        "# Drittanbieter-Lizenzen · PrepareAudio",
        "",
        f'PrepareAudio {root["version"]} steht unter der MIT-Lizenz (siehe LICENSE). Die App enthält die folgenden '
        f"{len(crates)} Softwarepakete (Stand dieses Builds, Ziel {a.target}). Die macOS-WebView wird vom System gestellt und ist nicht Teil der App.",
        "",
        "| Paket | Version | Lizenz | Quelle |",
        "|---|---|---|---|",
    ]
    for c in crates:
        lines.append(f'| {c["name"]} | {c["version"]} | {c["license"]} | {c["repository"]} |')
    lines += ["", "## Hinweise", ""]
    for c in crates:
        if c["note"]:
            lines.append(f'- **{c["name"]} {c["version"]}:** {c["note"]}')
    lines += ["", "## Lizenztexte", ""]
    for t in out["texts"]:
        lines += [f'### Gilt für: {", ".join(t["crates"])}', "", "```", t["text"], "```", ""]
    with open(os.path.join(ROOT, "THIRD_PARTY_LICENSES.md"), "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")
    missing = [c["name"] for c in crates if not c["texts"]]
    print(f"{len(crates)} crates, {len(out['texts'])} distinct license texts, without text: {missing}")


if __name__ == "__main__":
    sys.exit(main())
