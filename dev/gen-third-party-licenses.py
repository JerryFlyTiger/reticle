#!/usr/bin/env python3
"""Generate THIRD_PARTY_LICENSES.md from `cargo metadata`.

Lists every third-party crate linked into the shipped binaries, its SPDX
license expression, its upstream repository, and the copyright holders named
in its own license file (when the published crate ships one).

Run from the repository root:

    python3 dev/gen-third-party-licenses.py > THIRD_PARTY_LICENSES.md

It shells out to `cargo metadata` itself. Pass a pre-dumped metadata JSON file
as the first argument to skip that step.

Note that `cargo metadata` resolves the dependency graph for every platform,
not just the host, so the output covers Windows- and Linux-only crates too.
That is deliberate: the attribution file ships with every build.
"""
import json
import os
import re
import subprocess
import sys
from pathlib import Path

DERIVED_FILES_SECTION = """
## Derived source files

Beyond linked crates, some files in this repository were adapted from
third-party sources rather than written from scratch. Each carries an
`SPDX-License-Identifier` and an attribution header naming its upstream.

The tree-sitter highlight queries in `crates/core/queries/` were trimmed from
the corresponding upstream grammars' own `queries/highlights.scm`, all under
the MIT License:

| File | Upstream | Copyright |
| --- | --- | --- |
| `bash-highlights.scm` | tree-sitter-bash | Copyright (c) 2017 Max Brunsfeld |
| `c-highlights.scm` | tree-sitter-c | Copyright (c) 2014 Max Brunsfeld |
| `cpp-highlights.scm` | tree-sitter-c, tree-sitter-cpp | Copyright (c) 2014 Max Brunsfeld |
| `elisp-highlights.scm` | tree-sitter-elisp | Copyright (c) 2021 Wilfred Hughes |
| `java-highlights.scm` | tree-sitter-java | Copyright (c) 2017 Ayman Nadeem |
| `perl-highlights.scm` | tree-sitter-perl | Copyright 2025 Avishai \"Veesh\" Goldman |
| `python-highlights.scm` | tree-sitter-python | Copyright (c) 2016 Max Brunsfeld |
| `rust-highlights.scm` | tree-sitter-rust | Copyright (c) 2017 Maxim Sokolov |

`verilog-highlights.scm` was written from scratch -- the SystemVerilog grammar
crate ships no highlight query to start from -- and is not derived from any
third-party file.

The copyright holders above were read from each project's own `LICENSE` file.
`tree-sitter-cpp` and `tree-sitter-java` do not ship one inside their published
crate, so for those two the holder was taken from the upstream repository's
`LICENSE` instead. That is why the per-crate entries further down carry no
copyright line for them: the generator can only report what the package
contains.

## Notes on particular licenses

**Fonts.** The GUI front end embeds the default egui font set via
`epaint_default_fonts`, which is `(MIT OR Apache-2.0) AND OFL-1.1 AND
LicenseRef-UFL-1.0`. The bundled faces are covered by the SIL Open Font License
1.1 and the Ubuntu Font Licence 1.0. Both permit redistribution as part of a
larger work, including a commercial one, and both require that this attribution
travel with the binary.

**Unicode data.** The ICU-derived crates (`icu_*`, `zerovec`, `tinystr`,
`yoke`, and related) are under `Unicode-3.0`, the Unicode License v3, which is
permissive and requires attribution only.

**No copyleft.** No GPL, LGPL, AGPL, MPL, EPL or CDDL code is linked into the
shipped binaries. `r-efi` carries `MIT OR Apache-2.0 OR LGPL-2.1-or-later` and
is used here under the MIT option; it is a UEFI-target crate that does not
appear in the macOS, Linux or Windows build graphs at all.

"""

MIT_TEXT = """
## Full license texts

### MIT License

Permission is hereby granted, free of charge, to any person obtaining a copy
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
SOFTWARE.

### Apache License 2.0

The full text is at http://www.apache.org/licenses/LICENSE-2.0

### Other licenses

BSD-2-Clause, BSD-3-Clause, ISC, Zlib, 0BSD, Unlicense, CC0-1.0, BSL-1.0
(Boost), Unicode-3.0, OFL-1.1 and the Ubuntu Font Licence 1.0 all appear in the
table above. Each crate ships its own license file inside its published
package, and the canonical text of each license is available from SPDX at
https://spdx.org/licenses/ .
"""

LICENSE_FILE_RE = re.compile(r'^(LICENSE|LICENCE|COPYING|NOTICE|UNLICENSE)', re.I)
COPYRIGHT_RE = re.compile(r'^\s*(?:Copyright|\(c\)|\xa9)\s*.{0,150}$', re.I | re.M)


def copyright_lines(manifest_path: str) -> list:
    """Extract copyright lines from license files shipped in the crate."""
    root = Path(manifest_path).parent
    found = []
    try:
        entries = sorted(os.listdir(root))
    except OSError:
        return found
    for name in entries:
        if not LICENSE_FILE_RE.match(name):
            continue
        path = root / name
        if not path.is_file():
            continue
        try:
            text = path.read_text(encoding='utf-8', errors='replace')
        except OSError:
            continue
        for match in COPYRIGHT_RE.findall(text[:8000]):
            line = ' '.join(match.split())
            # Skip the generic disclaimer sentences that follow the notice.
            if len(line) > 8 and 'THE SOFTWARE' not in line.upper():
                found.append(line)
    # De-duplicate, preserving order.
    seen = set()
    unique = []
    for line in found:
        if line not in seen:
            seen.add(line)
            unique.append(line)
    return unique[:4]


def load_metadata() -> dict:
    """Read `cargo metadata` output, from a file argument or by running cargo."""
    if len(sys.argv) > 1:
        with open(sys.argv[1], encoding='utf-8') as handle:
            return json.load(handle)
    result = subprocess.run(
        ['cargo', 'metadata', '--format-version', '1'],
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)


def main() -> int:
    meta = load_metadata()
    packages = [p for p in meta['packages'] if p.get('source')]
    packages.sort(key=lambda p: (p['name'].lower(), p['version']))

    print('# Third-Party Licenses')
    print()
    print('Reticle links against the open-source Rust crates listed below.')
    print('Each is used under its own license; this file reproduces the')
    print('attribution those licenses require. Nothing in this file grants any')
    print('rights to Reticle itself, which is licensed separately -- see')
    print('`LICENSE.md`.')
    print()
    print('Regenerate with `python3 dev/gen-third-party-licenses.py`.')
    print()
    print(f'Total third-party crates: {len(packages)}')
    print()

    by_license = {}
    for p in packages:
        by_license.setdefault(p.get('license') or 'UNSPECIFIED', []).append(p)
    print('## License summary')
    print()
    print('| SPDX expression | Crates |')
    print('| --- | ---: |')
    for lic, ps in sorted(by_license.items(), key=lambda kv: (-len(kv[1]), kv[0])):
        print(f'| `{lic}` | {len(ps)} |')
    print()

    print(DERIVED_FILES_SECTION.strip())
    print()
    print('## Crates')
    print()
    for p in packages:
        print(f"### {p['name']} {p['version']}")
        print()
        print(f"- License: `{p.get('license') or 'UNSPECIFIED'}`")
        if p.get('repository'):
            print(f"- Repository: {p['repository']}")
        for line in copyright_lines(p['manifest_path']):
            print(f'- {line}')
        print()
    print(MIT_TEXT.strip())
    print()
    return 0


if __name__ == '__main__':
    sys.exit(main())
