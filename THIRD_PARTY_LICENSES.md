# Third-Party Licenses

Reticle links against the open-source Rust crates listed below.
Each is used under its own license; this file reproduces the
attribution those licenses require. Nothing in this file grants any
rights to Reticle itself, which is licensed separately -- see
`LICENSE.md`.

Regenerate with `python3 dev/gen-third-party-licenses.py`.

Total third-party crates: 454

## License summary

| SPDX expression | Crates |
| --- | ---: |
| `MIT OR Apache-2.0` | 194 |
| `MIT` | 105 |
| `Apache-2.0 OR MIT` | 33 |
| `Apache-2.0 WITH LLVM-exception` | 19 |
| `Unicode-3.0` | 18 |
| `MIT/Apache-2.0` | 17 |
| `Apache-2.0` | 14 |
| `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 7 |
| `Zlib OR Apache-2.0 OR MIT` | 7 |
| `MIT OR Apache-2.0 OR Zlib` | 5 |
| `Unlicense OR MIT` | 5 |
| `Zlib` | 3 |
| `Apache-2.0/MIT` | 2 |
| `BSD-2-Clause OR Apache-2.0 OR MIT` | 2 |
| `BSD-3-Clause` | 2 |
| `BSD-3-Clause OR Apache-2.0` | 2 |
| `BSD-3-Clause OR MIT OR Apache-2.0` | 2 |
| `BSL-1.0` | 2 |
| `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | 2 |
| `Unlicense/MIT` | 2 |
| `(MIT OR Apache-2.0) AND OFL-1.1 AND LicenseRef-UFL-1.0` | 1 |
| `(MIT OR Apache-2.0) AND Unicode-3.0` | 1 |
| `0BSD OR MIT OR Apache-2.0` | 1 |
| `Apache-2.0 / MIT` | 1 |
| `Apache-2.0 AND MIT` | 1 |
| `BSD-2-Clause` | 1 |
| `BSD-2-Clause OR MIT OR Apache-2.0` | 1 |
| `CC0-1.0` | 1 |
| `ISC` | 1 |
| `MIT / Apache-2.0` | 1 |
| `MIT OR Zlib OR Apache-2.0` | 1 |

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
| `perl-highlights.scm` | tree-sitter-perl | Copyright 2025 Avishai "Veesh" Goldman |
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

## Crates

### ab_glyph 0.2.32

- License: `Apache-2.0`
- Repository: https://github.com/alexheretic/ab-glyph
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### ab_glyph_rasterizer 0.1.10

- License: `Apache-2.0`
- Repository: https://github.com/alexheretic/ab-glyph
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### accesskit 0.16.3

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/AccessKit/accesskit

### accesskit_atspi_common 0.9.3

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/AccessKit/accesskit

### accesskit_consumer 0.24.3

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/AccessKit/accesskit

### accesskit_macos 0.17.4

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/AccessKit/accesskit

### accesskit_unix 0.12.3

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/AccessKit/accesskit

### accesskit_windows 0.23.2

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/AccessKit/accesskit

### accesskit_winit 0.22.4

- License: `Apache-2.0`
- Repository: https://github.com/AccessKit/accesskit

### adler2 2.0.1

- License: `0BSD OR MIT OR Apache-2.0`
- Repository: https://github.com/oyvindln/adler2
- Copyright (C) Jonas Schievink <jonasschievink@gmail.com>
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### ahash 0.8.12

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/tkaitchuck/ahash
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2018 Tom Kaitchuck

### aho-corasick 1.1.4

- License: `Unlicense OR MIT`
- Repository: https://github.com/BurntSushi/aho-corasick
- Copyright (c) 2015 Andrew Gallant

### allocator-api2 0.2.21

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/zakarumych/allocator-api2
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### android-activity 0.6.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-mobile/android-activity
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### android-properties 0.2.2

- License: `MIT`
- Repository: https://github.com/miklelappo/android-properties
- Copyright (c) 2020 Mikhail Lappo

### android_system_properties 0.1.5

- License: `MIT/Apache-2.0`
- Repository: https://github.com/nical/android_system_properties
- Copyright 2016 Nicolas Silva
- Copyright (c) 2013 Nicolas Silva
- COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER

### anyhow 1.0.104

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/dtolnay/anyhow
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### arbitrary 1.4.2

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-fuzz/arbitrary/
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2019 Manish Goregaokar

### arboard 3.6.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/1Password/arboard
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2022 The Arboard contributors

### arrayref 0.3.9

- License: `BSD-2-Clause`
- Repository: https://github.com/droundy/arrayref
- Copyright (c) 2015 David Roundy <roundyd@physics.oregonstate.edu>

### arrayvec 0.7.8

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/bluss/arrayvec
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Ulrik Sverdrup "bluss" 2015-2023

### as-raw-xcb-connection 1.0.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/psychon/as-raw-xcb-connection
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright 2019 as-raw-xcb-connection Contributers

### ash 0.38.0+1.3.281

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/ash-rs/ash
- Copyright (c) 2016 ASH

### async-broadcast 0.7.2

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/smol-rs/async-broadcast
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2020 Yoshua Wuyts

### async-channel 2.5.0

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/smol-rs/async-channel
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### async-executor 1.14.0

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/smol-rs/async-executor
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### async-fs 2.2.0

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/smol-rs/async-fs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### async-io 2.6.0

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/smol-rs/async-io
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### async-lock 3.4.2

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/smol-rs/async-lock
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### async-process 2.5.0

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/smol-rs/async-process
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### async-recursion 1.1.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/dcchut/async-recursion
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### async-signal 0.2.14

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/smol-rs/async-signal
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### async-task 4.7.1

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/smol-rs/async-task
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### async-trait 0.1.91

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/dtolnay/async-trait
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### atomic-waker 1.1.2

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/smol-rs/atomic-waker
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2016 Alex Crichton

### atspi 0.22.0

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/odilia-app/atspi
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2022 Tait Hoyem <tait@tait.tech>

### atspi-common 0.6.0

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/odilia-app/atspi
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2022 Tait Hoyem <tait@tait.tech>

### atspi-connection 0.6.0

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/odilia-app/atspi/
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2022 Tait Hoyem <tait@tait.tech>

### atspi-proxies 0.6.0

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/odilia-app/atspi

### autocfg 1.5.1

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/cuviper/autocfg
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2018 Josh Stone

### base64 0.21.7

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/marshallpierce/rust-base64
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2015 Alice Maz

### bit-set 0.6.0

- License: `MIT/Apache-2.0`
- Repository: https://github.com/contain-rs/bit-set
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2023 The Rust Project Developers

### bit-vec 0.7.0

- License: `MIT/Apache-2.0`
- Repository: https://github.com/contain-rs/bit-vec
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2023 The Rust Project Developers

### bitflags 1.3.2

- License: `MIT/Apache-2.0`
- Repository: https://github.com/bitflags/bitflags
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 The Rust Project Developers

### bitflags 2.13.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/bitflags/bitflags
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 The Rust Project Developers

### block 0.1.6

- License: `MIT`
- Repository: http://github.com/SSheldon/rust-block

### block-buffer 0.10.4

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/RustCrypto/utils
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2018-2019 The RustCrypto Project Developers

### block2 0.5.1

- License: `MIT`
- Repository: https://github.com/madsmtm/objc2

### blocking 1.6.2

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/smol-rs/blocking
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### bumpalo 3.20.3

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/fitzgen/bumpalo
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2019 Nick Fitzgerald

### bytemuck 1.25.2

- License: `Zlib OR Apache-2.0 OR MIT`
- Repository: https://github.com/Lokathor/bytemuck
- Copyright (c) 2019 Daniel "Lokathor" Gee.

### bytemuck_derive 1.11.0

- License: `Zlib OR Apache-2.0 OR MIT`
- Repository: https://github.com/Lokathor/bytemuck
- Copyright (c) 2019 Daniel "Lokathor" Gee.

### byteorder-lite 0.1.0

- License: `Unlicense OR MIT`
- Repository: https://github.com/image-rs/byteorder-lite
- Copyright (c) 2015 Andrew Gallant

### bytes 1.12.1

- License: `MIT`
- Repository: https://github.com/tokio-rs/bytes
- Copyright (c) 2018 Carl Lerche

### calloop 0.13.0

- License: `MIT`
- Repository: https://github.com/Smithay/calloop
- Copyright (c) 2018 Victor Berger

### calloop 0.14.4

- License: `MIT`
- Repository: https://github.com/Smithay/calloop
- Copyright (c) 2018 Victor Berger

### calloop-wayland-source 0.3.0

- License: `MIT`
- Repository: https://github.com/smithay/calloop-wayland-source
- Copyright (c) 2023 Kirill Chibisov

### calloop-wayland-source 0.4.1

- License: `MIT`
- Repository: https://github.com/smithay/calloop-wayland-source
- Copyright (c) 2023 Kirill Chibisov

### cc 1.3.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-lang/cc-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 Alex Crichton

### cfg-if 1.0.4

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-lang/cfg-if
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 Alex Crichton

### cfg_aliases 0.1.1

- License: `MIT`
- Repository: https://github.com/katharostech/cfg_aliases
- Copyright (c) 2020 Katharos Technology

### cfg_aliases 0.2.2

- License: `MIT`
- Repository: https://github.com/katharostech/cfg_aliases
- Copyright (c) 2020 Katharos Technology

### cgl 0.3.2

- License: `MIT / Apache-2.0`
- Repository: https://github.com/servo/cgl-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2012-2013 Mozilla Foundation

### clipboard-win 5.4.1

- License: `BSL-1.0`
- Repository: https://github.com/DoumanAsh/clipboard-win

### codespan-reporting 0.11.1

- License: `Apache-2.0`
- Repository: https://github.com/brendanzab/codespan

### com 0.6.0

- License: `MIT`
- Repository: https://github.com/microsoft/com-rs
- Copyright (c) Microsoft Corporation.

### com_macros 0.6.0

- License: `MIT`
- Repository: https://github.com/microsoft/com-rs

### com_macros_support 0.6.0

- License: `MIT`
- Repository: https://github.com/microsoft/com-rs

### combine 4.6.7

- License: `MIT`
- Repository: https://github.com/Marwes/combine
- Copyright (c) 2015 Markus Westerlind

### concurrent-queue 2.5.0

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/smol-rs/concurrent-queue
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### core-foundation 0.10.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/servo/core-foundation-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2012-2013 Mozilla Foundation

### core-foundation 0.9.4

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/servo/core-foundation-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2012-2013 Mozilla Foundation

### core-foundation-sys 0.8.7

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/servo/core-foundation-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2012-2013 Mozilla Foundation

### core-graphics 0.23.2

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/servo/core-foundation-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2012-2013 Mozilla Foundation

### core-graphics-types 0.1.3

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/servo/core-foundation-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2012-2013 Mozilla Foundation

### cpufeatures 0.2.17

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/RustCrypto/utils
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2020-2025 The RustCrypto Project Developers

### cranelift-assembler-x64 0.133.1

- License: `Apache-2.0 WITH LLVM-exception`

### cranelift-assembler-x64-meta 0.133.1

- License: `Apache-2.0 WITH LLVM-exception`

### cranelift-bforest 0.133.1

- License: `Apache-2.0 WITH LLVM-exception`
- Repository: https://github.com/bytecodealliance/wasmtime
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### cranelift-bitset 0.133.1

- License: `Apache-2.0 WITH LLVM-exception`
- Repository: https://github.com/bytecodealliance/wasmtime

### cranelift-codegen 0.133.1

- License: `Apache-2.0 WITH LLVM-exception`
- Repository: https://github.com/bytecodealliance/wasmtime
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### cranelift-codegen-meta 0.133.1

- License: `Apache-2.0 WITH LLVM-exception`
- Repository: https://github.com/bytecodealliance/wasmtime
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### cranelift-codegen-shared 0.133.1

- License: `Apache-2.0 WITH LLVM-exception`
- Repository: https://github.com/bytecodealliance/wasmtime
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### cranelift-control 0.133.1

- License: `Apache-2.0 WITH LLVM-exception`
- Repository: https://github.com/bytecodealliance/wasmtime
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### cranelift-entity 0.133.1

- License: `Apache-2.0 WITH LLVM-exception`
- Repository: https://github.com/bytecodealliance/wasmtime
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### cranelift-frontend 0.133.1

- License: `Apache-2.0 WITH LLVM-exception`
- Repository: https://github.com/bytecodealliance/wasmtime
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### cranelift-isle 0.133.1

- License: `Apache-2.0 WITH LLVM-exception`
- Repository: https://github.com/bytecodealliance/wasmtime/tree/main/cranelift/isle

### cranelift-jit 0.133.1

- License: `Apache-2.0 WITH LLVM-exception`
- Repository: https://github.com/bytecodealliance/wasmtime
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### cranelift-module 0.133.1

- License: `Apache-2.0 WITH LLVM-exception`
- Repository: https://github.com/bytecodealliance/wasmtime
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### cranelift-native 0.133.1

- License: `Apache-2.0 WITH LLVM-exception`
- Repository: https://github.com/bytecodealliance/wasmtime
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### cranelift-srcgen 0.133.1

- License: `Apache-2.0 WITH LLVM-exception`
- Repository: https://github.com/bytecodealliance/wasmtime

### crc32fast 1.5.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/srijs/rust-crc32fast
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2018 Sam Rijs, Alex Crichton and contributors

### crossbeam-utils 0.8.22

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/crossbeam-rs/crossbeam
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2019 The Crossbeam Project Developers

### crossterm 0.28.1

- License: `MIT`
- Repository: https://github.com/crossterm-rs/crossterm
- Copyright (c) 2019 Timon

### crossterm_winapi 0.9.1

- License: `MIT`
- Repository: https://github.com/crossterm-rs/crossterm-winapi
- Copyright (c) 2019 Timon

### crypto-common 0.1.7

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/RustCrypto/traits
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2021 RustCrypto Developers

### cursor-icon 1.2.0

- License: `MIT OR Apache-2.0 OR Zlib`
- Repository: https://github.com/rust-windowing/cursor-icon
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2023 Kirill Chibisov

### digest 0.10.7

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/RustCrypto/traits
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2017 Artyom Pavlov

### dispatch 0.2.0

- License: `MIT`
- Repository: http://github.com/SSheldon/rust-dispatch

### dispatch2 0.3.1

- License: `Zlib OR Apache-2.0 OR MIT`
- Repository: https://github.com/madsmtm/objc2

### displaydoc 0.2.6

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/yaahc/displaydoc
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### dlib 0.5.3

- License: `MIT`
- Repository: https://github.com/elinorbgr/dlib
- Copyright (c) 2015 Victor Berger

### document-features 0.2.12

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/slint-ui/document-features
- Copyright (c) 2020 Olivier Goffart <ogoffart@sixtyfps.io>

### downcast-rs 1.2.1

- License: `MIT/Apache-2.0`
- Repository: https://github.com/marcianx/downcast-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2020 Ashish Myles and contributors

### dpi 0.1.2

- License: `Apache-2.0 AND MIT`
- Repository: https://github.com/rust-windowing/winit
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2018 Jorge Aparicio

### ecolor 0.29.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/emilk/egui

### eframe 0.29.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/emilk/egui/tree/master/crates/eframe

### egui 0.29.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/emilk/egui

### egui-wgpu 0.29.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/emilk/egui/tree/master/crates/egui-wgpu

### egui-winit 0.29.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/emilk/egui/tree/master/crates/egui-winit

### egui_glow 0.29.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/emilk/egui/tree/master/crates/egui_glow

### emath 0.29.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/emilk/egui/tree/master/crates/emath

### endi 1.1.1

- License: `MIT`
- Repository: https://github.com/zeenix/endi

### enumflags2 0.7.12

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/meithecatte/enumflags2
- Copyright (c) 2017-2023 Maik Klein, Maja Kądziołka

### enumflags2_derive 0.7.12

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/meithecatte/enumflags2
- Copyright (c) 2017 Maik Klein

### enumn 0.1.14

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/dtolnay/enumn
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### epaint 0.29.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/emilk/egui/tree/master/crates/epaint

### epaint_default_fonts 0.29.1

- License: `(MIT OR Apache-2.0) AND OFL-1.1 AND LicenseRef-UFL-1.0`
- Repository: https://github.com/emilk/egui/tree/master/crates/epaint_default_fonts

### equivalent 1.0.2

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/indexmap-rs/equivalent
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2016--2023

### errno 0.3.14

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/lambda-fairy/rust-errno
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 Chris Wong

### error-code 3.3.2

- License: `BSL-1.0`
- Repository: https://github.com/DoumanAsh/error-code

### event-listener 5.4.1

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/smol-rs/event-listener
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### event-listener-strategy 0.5.4

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/smol-rs/event-listener-strategy
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### fastrand 2.5.0

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/smol-rs/fastrand
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### fdeflate 0.3.7

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/image-rs/fdeflate
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### find-msvc-tools 0.1.9

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-lang/cc-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 Alex Crichton

### flate2 1.1.9

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-lang/flate2-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014-2026 Alex Crichton

### fnv 1.0.7

- License: `Apache-2.0 / MIT`
- Repository: https://github.com/servo/rust-fnv
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2017 Contributors

### foldhash 0.1.5

- License: `Zlib`
- Repository: https://github.com/orlp/foldhash
- Copyright (c) 2024 Orson Peters

### foldhash 0.2.0

- License: `Zlib`
- Repository: https://github.com/orlp/foldhash
- Copyright (c) 2024 Orson Peters

### foreign-types 0.5.0

- License: `MIT/Apache-2.0`
- Repository: https://github.com/sfackler/foreign-types
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2017 The foreign-types Developers

### foreign-types-macros 0.2.3

- License: `MIT/Apache-2.0`
- Repository: https://github.com/sfackler/foreign-types
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2017 The foreign-types Developers

### foreign-types-shared 0.3.1

- License: `MIT/Apache-2.0`
- Repository: https://github.com/sfackler/foreign-types
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2017 The foreign-types Developers

### form_urlencoded 1.2.2

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/servo/rust-url
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2013-2016 The rust-url developers

### futures-core 0.3.33

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-lang/futures-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2016 Alex Crichton

### futures-io 0.3.33

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-lang/futures-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2016 Alex Crichton

### futures-lite 2.6.1

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/smol-rs/futures-lite
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2016 Alex Crichton

### futures-macro 0.3.33

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-lang/futures-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2016 Alex Crichton

### futures-sink 0.3.33

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-lang/futures-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2016 Alex Crichton

### futures-task 0.3.33

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-lang/futures-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2016 Alex Crichton

### futures-util 0.3.33

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-lang/futures-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2016 Alex Crichton

### generic-array 0.14.7

- License: `MIT`
- Repository: https://github.com/fizyk20/generic-array.git
- Copyright (c) 2015 Bartłomiej Kamiński

### gethostname 1.1.0

- License: `Apache-2.0`
- Repository: https://codeberg.org/swsnr/gethostname.rs.git
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### getrandom 0.2.17

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-random/getrandom
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2018-2024 The rust-random Project Developers

### getrandom 0.3.4

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-random/getrandom
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2018-2025 The rust-random Project Developers

### getrandom 0.4.3

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-random/getrandom
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2018-2026 The rust-random Project Developers

### gimli 0.33.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/gimli-rs/gimli
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2015 The Rust Project Developers

### gl_generator 0.14.0

- License: `Apache-2.0`
- Repository: https://github.com/brendanzab/gl-rs/

### glow 0.13.1

- License: `MIT OR Apache-2.0 OR Zlib`
- Repository: https://github.com/grovesNL/glow
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### glow 0.14.2

- License: `MIT OR Apache-2.0 OR Zlib`
- Repository: https://github.com/grovesNL/glow
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### glutin 0.32.3

- License: `Apache-2.0`
- Repository: https://github.com/rust-windowing/glutin
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### glutin-winit 0.5.0

- License: `MIT`
- Repository: https://github.com/rust-windowing/glutin
- Copyright © 2022 Kirill Chibisov

### glutin_egl_sys 0.7.1

- License: `Apache-2.0`
- Repository: https://github.com/rust-windowing/glutin
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### glutin_glx_sys 0.6.1

- License: `Apache-2.0`
- Repository: https://github.com/rust-windowing/glutin
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### glutin_wgl_sys 0.6.1

- License: `Apache-2.0`
- Repository: https://github.com/rust-windowing/glutin
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### gpu-alloc 0.6.2

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/zakarumych/gpu-alloc

### gpu-alloc-types 0.3.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/zakarumych/gpu-alloc

### gpu-allocator 0.26.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/Traverse-Research/gpu-allocator
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2021 Traverse Research B.V.

### gpu-descriptor 0.3.2

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/zakarumych/gpu-descriptor

### gpu-descriptor-types 0.2.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/zakarumych/gpu-descriptor

### hashbrown 0.15.5

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-lang/hashbrown
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2016 Amanieu d'Antras

### hashbrown 0.16.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-lang/hashbrown
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2016 Amanieu d'Antras

### hashbrown 0.17.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-lang/hashbrown
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2016 Amanieu d'Antras

### hassle-rs 0.11.0

- License: `MIT`
- Repository: https://github.com/Traverse-Research/hassle-rs
- Copyright (c) 2018 Jasper Bekkers

### heck 0.5.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/withoutboats/heck
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2015 The Rust Project Developers

### hermit-abi 0.5.2

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/hermit-os/hermit-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### hex 0.4.3

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/KokaKiwi/rust-hex
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2013-2014 The Rust Project Developers.

### hexf-parse 0.2.1

- License: `CC0-1.0`
- Repository: https://github.com/lifthrasiir/hexf

### home 0.5.12

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-lang/cargo
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### icu_collections 2.2.0

- License: `Unicode-3.0`
- Repository: https://github.com/unicode-org/icu4x
- COPYRIGHT AND PERMISSION NOTICE
- Copyright © 2020-2024 Unicode, Inc.

### icu_locale_core 2.2.0

- License: `Unicode-3.0`
- Repository: https://github.com/unicode-org/icu4x
- COPYRIGHT AND PERMISSION NOTICE
- Copyright © 2020-2024 Unicode, Inc.

### icu_normalizer 2.2.0

- License: `Unicode-3.0`
- Repository: https://github.com/unicode-org/icu4x
- COPYRIGHT AND PERMISSION NOTICE
- Copyright © 2020-2024 Unicode, Inc.

### icu_normalizer_data 2.2.0

- License: `Unicode-3.0`
- Repository: https://github.com/unicode-org/icu4x
- COPYRIGHT AND PERMISSION NOTICE
- Copyright © 2020-2024 Unicode, Inc.

### icu_properties 2.2.0

- License: `Unicode-3.0`
- Repository: https://github.com/unicode-org/icu4x
- COPYRIGHT AND PERMISSION NOTICE
- Copyright © 2020-2024 Unicode, Inc.

### icu_properties_data 2.2.0

- License: `Unicode-3.0`
- Repository: https://github.com/unicode-org/icu4x
- COPYRIGHT AND PERMISSION NOTICE
- Copyright © 2020-2024 Unicode, Inc.

### icu_provider 2.2.0

- License: `Unicode-3.0`
- Repository: https://github.com/unicode-org/icu4x
- COPYRIGHT AND PERMISSION NOTICE
- Copyright © 2020-2024 Unicode, Inc.

### idna 1.1.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/servo/rust-url/
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2013-2025 The rust-url developers

### idna_adapter 1.2.2

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/hsivonen/idna_adapter
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) The rust-url developers

### image 0.25.10

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/image-rs/image
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### immutable-chunkmap 2.1.3

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/estokes/immutable-chunkmap
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright 2022 Eric Stokes

### indexmap 2.14.0

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/indexmap-rs/indexmap
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2016--2017

### itoa 1.0.18

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/dtolnay/itoa
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### jni 0.22.4

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/jni-rs/jni-rs

### jni-macros 0.22.4

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/jni-rs/jni-rs

### jni-sys 0.3.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/jni-rs/jni-sys
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2015 The rust-jni-sys Developers

### jni-sys 0.4.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/jni-rs/jni-sys
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2015 The rust-jni-sys Developers

### jni-sys-macros 0.4.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/jni-rs/jni-sys

### jobserver 0.1.35

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-lang/jobserver-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 Alex Crichton

### js-sys 0.3.103

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/js-sys
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 Alex Crichton

### khronos-egl 6.0.0

- License: `MIT/Apache-2.0`
- Repository: https://github.com/timothee-haudebourg/khronos-egl
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### khronos_api 3.1.0

- License: `Apache-2.0`
- Repository: https://github.com/brendanzab/gl-rs/

### libc 0.2.186

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-lang/libc
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) The Rust Project Developers

### libloading 0.8.9

- License: `ISC`
- Repository: https://github.com/nagisa/rust_libloading/
- Copyright © 2015, Simonas Kazlauskas

### libm 0.2.16

- License: `MIT`
- Repository: https://github.com/rust-lang/compiler-builtins
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### libredox 0.1.18

- License: `MIT`
- Repository: https://gitlab.redox-os.org/redox-os/libredox.git
- Copyright (c) 2023 4lDO2

### linux-raw-sys 0.12.1

- License: `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT`
- Repository: https://github.com/sunfishcode/linux-raw-sys
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### linux-raw-sys 0.4.15

- License: `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT`
- Repository: https://github.com/sunfishcode/linux-raw-sys
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### litemap 0.8.2

- License: `Unicode-3.0`
- Repository: https://github.com/unicode-org/icu4x
- COPYRIGHT AND PERMISSION NOTICE
- Copyright © 2020-2024 Unicode, Inc.

### litrs 1.0.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/LukasKalbertodt/litrs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2020 Project Developers

### lock_api 0.4.14

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/Amanieu/parking_lot
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2016 The Rust Project Developers

### log 0.4.33

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-lang/log
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 The Rust Project Developers

### mach2 0.4.3

- License: `BSD-2-Clause OR MIT OR Apache-2.0`
- Repository: https://github.com/JohnTitor/mach2
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2019 Nick Fitzgerald, 2021 Yuki Okushi

### malloc_buf 0.0.6

- License: `MIT`
- Repository: https://github.com/SSheldon/malloc_buf

### memchr 2.8.3

- License: `Unlicense OR MIT`
- Repository: https://github.com/BurntSushi/memchr
- Copyright (c) 2015 Andrew Gallant

### memmap2 0.2.3

- License: `MIT/Apache-2.0`
- Repository: https://github.com/RazrFalcon/memmap2-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2020 Evgeniy Reizner

### memmap2 0.9.11

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/RazrFalcon/memmap2-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2020 Yevhenii Reizner

### memoffset 0.9.1

- License: `MIT`
- Repository: https://github.com/Gilnaa/memoffset
- Copyright (c) 2017 Gilad Naaman

### metal 0.29.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/gfx-rs/metal-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2010 The Rust Project Developers

### miniz_oxide 0.8.9

- License: `MIT OR Zlib OR Apache-2.0`
- Repository: https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide
- Copyright 2013-2014 RAD Game Tools and Valve Software
- Copyright 2010-2014 Rich Geldreich and Tenacious Software LLC
- Copyright (c) 2017 Frommi
- Copyright (c) 2017-2024 oyvindln

### mio 1.2.2

- License: `MIT`
- Repository: https://github.com/tokio-rs/mio
- Copyright (c) 2014 Carl Lerche and other MIO contributors

### moxcms 0.8.1

- License: `BSD-3-Clause OR Apache-2.0`
- Repository: https://github.com/awxkee/moxcms.git
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Radzivon Bartoshyk. All rights reserved.

### naga 22.1.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/gfx-rs/wgpu/tree/trunk/naga

### ndk 0.9.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-mobile/ndk

### ndk-context 0.1.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-windowing/android-ndk-rs

### ndk-sys 0.5.0+25.2.9519653

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-mobile/ndk

### ndk-sys 0.6.0+11769913

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-mobile/ndk

### nix 0.29.0

- License: `MIT`
- Repository: https://github.com/nix-rust/nix
- Copyright (c) 2015 Carl Lerche + nix-rust Authors

### nohash-hasher 0.2.0

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/paritytech/nohash-hasher
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright 2018 Parity Technologies (UK) Ltd.

### num-bigint 0.4.8

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-num/num-bigint
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 The Rust Project Developers

### num-integer 0.1.46

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-num/num-integer
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 The Rust Project Developers

### num-traits 0.2.19

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-num/num-traits
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 The Rust Project Developers

### num_enum 0.7.6

- License: `BSD-3-Clause OR MIT OR Apache-2.0`
- Repository: https://github.com/illicitonion/num_enum
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2018, Daniel Wagner-Hall

### num_enum_derive 0.7.6

- License: `BSD-3-Clause OR MIT OR Apache-2.0`
- Repository: https://github.com/illicitonion/num_enum
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2018, Daniel Wagner-Hall

### objc 0.2.7

- License: `MIT`
- Repository: http://github.com/SSheldon/rust-objc
- Copyright (c) Steven Sheldon

### objc-sys 0.3.5

- License: `MIT`
- Repository: https://github.com/madsmtm/objc2

### objc2 0.5.2

- License: `MIT`
- Repository: https://github.com/madsmtm/objc2

### objc2 0.6.4

- License: `MIT`
- Repository: https://github.com/madsmtm/objc2

### objc2-app-kit 0.2.2

- License: `MIT`
- Repository: https://github.com/madsmtm/objc2

### objc2-app-kit 0.3.2

- License: `Zlib OR Apache-2.0 OR MIT`
- Repository: https://github.com/madsmtm/objc2

### objc2-cloud-kit 0.2.2

- License: `MIT`
- Repository: https://github.com/madsmtm/objc2

### objc2-contacts 0.2.2

- License: `MIT`
- Repository: https://github.com/madsmtm/objc2

### objc2-core-data 0.2.2

- License: `MIT`
- Repository: https://github.com/madsmtm/objc2

### objc2-core-foundation 0.3.2

- License: `Zlib OR Apache-2.0 OR MIT`
- Repository: https://github.com/madsmtm/objc2

### objc2-core-graphics 0.3.2

- License: `Zlib OR Apache-2.0 OR MIT`
- Repository: https://github.com/madsmtm/objc2

### objc2-core-image 0.2.2

- License: `MIT`
- Repository: https://github.com/madsmtm/objc2

### objc2-core-location 0.2.2

- License: `MIT`
- Repository: https://github.com/madsmtm/objc2

### objc2-encode 4.1.0

- License: `MIT`
- Repository: https://github.com/madsmtm/objc2

### objc2-foundation 0.2.2

- License: `MIT`
- Repository: https://github.com/madsmtm/objc2

### objc2-foundation 0.3.2

- License: `MIT`
- Repository: https://github.com/madsmtm/objc2

### objc2-io-surface 0.3.2

- License: `Zlib OR Apache-2.0 OR MIT`
- Repository: https://github.com/madsmtm/objc2

### objc2-link-presentation 0.2.2

- License: `MIT`
- Repository: https://github.com/madsmtm/objc2

### objc2-metal 0.2.2

- License: `MIT`
- Repository: https://github.com/madsmtm/objc2

### objc2-quartz-core 0.2.2

- License: `MIT`
- Repository: https://github.com/madsmtm/objc2

### objc2-symbols 0.2.2

- License: `MIT`
- Repository: https://github.com/madsmtm/objc2

### objc2-ui-kit 0.2.2

- License: `MIT`
- Repository: https://github.com/madsmtm/objc2

### objc2-uniform-type-identifiers 0.2.2

- License: `MIT`
- Repository: https://github.com/madsmtm/objc2

### objc2-user-notifications 0.2.2

- License: `MIT`
- Repository: https://github.com/madsmtm/objc2

### once_cell 1.21.4

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/matklad/once_cell
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### orbclient 0.3.55

- License: `MIT`
- Repository: https://gitlab.redox-os.org/redox-os/orbclient
- Copyright (c) 2015-2019 Jeremy Soller

### ordered-stream 0.2.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/danieldg/ordered-stream
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### owned_ttf_parser 0.25.1

- License: `Apache-2.0`
- Repository: https://github.com/alexheretic/owned-ttf-parser
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### parking 2.2.1

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/smol-rs/parking
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright 2014-2020 The Rust Project Developers

### parking_lot 0.12.5

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/Amanieu/parking_lot
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2016 The Rust Project Developers

### parking_lot_core 0.9.12

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/Amanieu/parking_lot
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2016 The Rust Project Developers

### paste 1.0.15

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/dtolnay/paste
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### percent-encoding 2.3.2

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/servo/rust-url/
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2013-2025 The rust-url developers

### pin-project 1.1.13

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/taiki-e/pin-project
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### pin-project-internal 1.1.13

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/taiki-e/pin-project
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### pin-project-lite 0.2.17

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/taiki-e/pin-project-lite
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### piper 0.2.5

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/smol-rs/piper
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### pkg-config 0.3.33

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-lang/pkg-config-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 Alex Crichton

### plain 0.2.3

- License: `MIT/Apache-2.0`
- Repository: https://github.com/randomites/plain
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2017 Plain contributors

### png 0.18.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/image-rs/image-png
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2015 nwin

### polling 3.11.0

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/smol-rs/polling
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### potential_utf 0.1.5

- License: `Unicode-3.0`
- Repository: https://github.com/unicode-org/icu4x
- COPYRIGHT AND PERMISSION NOTICE
- Copyright © 2020-2024 Unicode, Inc.

### ppv-lite86 0.2.21

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/cryptocorrosion/cryptocorrosion
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2019 The CryptoCorrosion Contributors

### presser 0.3.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/EmbarkStudios/presser
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2019 Embark Studios

### proc-macro-crate 3.5.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/bkchr/proc-macro-crate
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### proc-macro2 1.0.107

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/dtolnay/proc-macro2
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### profiling 1.0.18

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/aclysma/profiling

### pxfm 0.1.30

- License: `BSD-3-Clause OR Apache-2.0`
- Repository: https://github.com/awxkee/pxfm
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Radzivon Bartoshyk. All rights reserved.

### quick-xml 0.30.0

- License: `MIT`
- Repository: https://github.com/tafia/quick-xml
- Copyright (c) 2016 Johann Tuffe

### quick-xml 0.39.4

- License: `MIT`
- Repository: https://github.com/tafia/quick-xml
- Copyright (c) 2016 Johann Tuffe

### quote 1.0.47

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/dtolnay/quote
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### r-efi 5.3.0

- License: `MIT OR Apache-2.0 OR LGPL-2.1-or-later`
- Repository: https://github.com/r-efi/r-efi

### r-efi 6.0.0

- License: `MIT OR Apache-2.0 OR LGPL-2.1-or-later`
- Repository: https://github.com/r-efi/r-efi

### rand 0.8.7

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-random/rand
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright 2018 Developers of the Rand project

### rand_chacha 0.3.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-random/rand
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright 2018 Developers of the Rand project

### rand_core 0.6.4

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-random/rand
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright 2018 Developers of the Rand project

### raw-window-handle 0.6.2

- License: `MIT OR Apache-2.0 OR Zlib`
- Repository: https://github.com/rust-windowing/raw-window-handle
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2019 Osspial

### redox_syscall 0.4.1

- License: `MIT`
- Repository: https://gitlab.redox-os.org/redox-os/syscall
- Copyright (c) 2017 Redox OS Developers

### redox_syscall 0.5.18

- License: `MIT`
- Repository: https://gitlab.redox-os.org/redox-os/syscall
- Copyright (c) 2017 Redox OS Developers

### redox_syscall 0.9.0

- License: `MIT`
- Repository: https://gitlab.redox-os.org/redox-os/syscall
- Copyright (c) 2017 Redox OS Developers

### regalloc2 0.15.1

- License: `Apache-2.0 WITH LLVM-exception`
- Repository: https://github.com/bytecodealliance/regalloc2
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### regex 1.13.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-lang/regex
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 The Rust Project Developers

### regex-automata 0.4.16

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-lang/regex
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 The Rust Project Developers

### regex-syntax 0.8.11

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-lang/regex
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 The Rust Project Developers

### region 3.0.2

- License: `MIT`
- Repository: https://github.com/darfink/region-rs
- Copyright (c) 2016 Elliott Linder

### renderdoc-sys 1.1.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/ebkalderon/renderdoc-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2022 Eyal Kalderon

### ron 0.8.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/ron-rs/ron
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2017 RON developers

### rustc-hash 1.1.0

- License: `Apache-2.0/MIT`
- Repository: https://github.com/rust-lang-nursery/rustc-hash
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### rustc-hash 2.1.3

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/rust-lang/rustc-hash
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### rustc_version 0.4.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/djc/rustc-version-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2016 The Rust Project Developers

### rustix 0.38.44

- License: `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT`
- Repository: https://github.com/bytecodealliance/rustix
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### rustix 1.1.4

- License: `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT`
- Repository: https://github.com/bytecodealliance/rustix
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### rustversion 1.0.23

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/dtolnay/rustversion
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### same-file 1.0.6

- License: `Unlicense/MIT`
- Repository: https://github.com/BurntSushi/same-file
- Copyright (c) 2017 Andrew Gallant

### scoped-tls 1.0.1

- License: `MIT/Apache-2.0`
- Repository: https://github.com/alexcrichton/scoped-tls
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 Alex Crichton

### scopeguard 1.2.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/bluss/scopeguard
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2016-2019 Ulrik Sverdrup "bluss" and scopeguard developers

### sctk-adwaita 0.10.1

- License: `MIT`
- Repository: https://github.com/PolyMeilex/sctk-adwaita
- Copyright (c) 2022 Bartłomiej Maryńczak

### semver 1.0.28

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/dtolnay/semver
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### serde 1.0.229

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/serde-rs/serde
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### serde_core 1.0.229

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/serde-rs/serde
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### serde_derive 1.0.229

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/serde-rs/serde
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### serde_json 1.0.151

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/serde-rs/json
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### serde_repr 0.1.21

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/dtolnay/serde-repr
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### sha1 0.10.7

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/RustCrypto/hashes
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2006-2009 Graydon Hoare

### shlex 2.0.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/comex/rust-shlex
- Copyright 2015 Nicholas Allegra (comex).
- Copyright (c) 2015 Nicholas Allegra (comex).

### signal-hook 0.3.18

- License: `Apache-2.0/MIT`
- Repository: https://github.com/vorner/signal-hook
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2017 tokio-jsonrpc developers

### signal-hook-mio 0.2.5

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/vorner/signal-hook
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2017 tokio-jsonrpc developers

### signal-hook-registry 1.4.8

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/vorner/signal-hook
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2017 tokio-jsonrpc developers

### simd-adler32 0.3.10

- License: `MIT`
- Repository: https://github.com/mcountryman/simd-adler32
- Copyright (c) [2021] [Marvin Countryman]

### simd_cesu8 1.2.0

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/seancroach/simd_cesu8
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### simdutf8 0.1.5

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rusticstuff/simdutf8
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### slab 0.4.12

- License: `MIT`
- Repository: https://github.com/tokio-rs/slab
- Copyright (c) 2019 Carl Lerche

### slotmap 1.1.1

- License: `Zlib`
- Repository: https://github.com/orlp/slotmap
- Copyright (c) 2021 Orson Peters <orsonpeters@gmail.com>

### smallvec 1.15.2

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/servo/rust-smallvec
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2018 The Servo Project Developers

### smithay-client-toolkit 0.19.2

- License: `MIT`
- Repository: https://github.com/smithay/client-toolkit
- Copyright (c) 2018 Victor Berger

### smithay-client-toolkit 0.20.0

- License: `MIT`
- Repository: https://github.com/smithay/client-toolkit
- Copyright (c) 2018 Victor Berger

### smithay-clipboard 0.7.3

- License: `MIT`
- Repository: https://github.com/smithay/smithay-clipboard
- Copyright (c) 2018 Lucas Timmins & Victor Berger

### smol_str 0.2.2

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/rust-analyzer/smol_str
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### spirv 0.3.0+sdk-1.3.268.0

- License: `Apache-2.0`
- Repository: https://github.com/gfx-rs/rspirv

### stable_deref_trait 1.2.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/storyyeller/stable_deref_trait
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2017 Robert Grosse

### static_assertions 1.1.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/nvzqz/static-assertions-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2017 Nikolai Vazquez

### streaming-iterator 0.1.9

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/sfackler/streaming-iterator
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2016 Steven Fackler

### strict-num 0.1.1

- License: `MIT`
- Repository: https://github.com/RazrFalcon/strict-num
- Copyright (c) 2022 Yevhenii Reizner

### syn 1.0.109

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/dtolnay/syn
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### syn 2.0.119

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/dtolnay/syn
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### syn 3.0.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/dtolnay/syn
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### synstructure 0.13.2

- License: `MIT`
- Repository: https://github.com/mystor/synstructure
- Copyright 2016 Nika Layzell

### target-lexicon 0.13.5

- License: `Apache-2.0 WITH LLVM-exception`
- Repository: https://github.com/bytecodealliance/target-lexicon
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### tempfile 3.27.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/Stebalien/tempfile
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2015 Steven Allen

### termcolor 1.4.1

- License: `Unlicense OR MIT`
- Repository: https://github.com/BurntSushi/termcolor
- Copyright (c) 2015 Andrew Gallant

### thiserror 1.0.69

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/dtolnay/thiserror
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### thiserror 2.0.19

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/dtolnay/thiserror
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### thiserror-impl 1.0.69

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/dtolnay/thiserror
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### thiserror-impl 2.0.19

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/dtolnay/thiserror
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### tiny-skia 0.11.4

- License: `BSD-3-Clause`
- Repository: https://github.com/RazrFalcon/tiny-skia
- Copyright (c) 2011 Google Inc. All rights reserved.
- Copyright (c) 2020 Yevhenii Reizner All rights reserved.

### tiny-skia-path 0.11.4

- License: `BSD-3-Clause`
- Repository: https://github.com/RazrFalcon/tiny-skia/tree/master/path
- Copyright (c) 2011 Google Inc. All rights reserved.
- Copyright (c) 2020 Yevhenii Reizner All rights reserved.

### tinystr 0.8.3

- License: `Unicode-3.0`
- Repository: https://github.com/unicode-org/icu4x
- COPYRIGHT AND PERMISSION NOTICE
- Copyright © 2020-2024 Unicode, Inc.

### toml_datetime 1.1.1+spec-1.1.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/toml-rs/toml
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Individual contributors

### toml_edit 0.25.13+spec-1.1.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/toml-rs/toml
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Individual contributors

### toml_parser 1.1.2+spec-1.1.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/toml-rs/toml
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Individual contributors

### tracing 0.1.44

- License: `MIT`
- Repository: https://github.com/tokio-rs/tracing
- Copyright (c) 2019 Tokio Contributors

### tracing-attributes 0.1.31

- License: `MIT`
- Repository: https://github.com/tokio-rs/tracing
- Copyright (c) 2019 Tokio Contributors

### tracing-core 0.1.36

- License: `MIT`
- Repository: https://github.com/tokio-rs/tracing
- Copyright (c) 2019 Tokio Contributors

### tree-sitter 0.25.10

- License: `MIT`
- Repository: https://github.com/tree-sitter/tree-sitter

### tree-sitter-bash 0.25.1

- License: `MIT`
- Repository: https://github.com/tree-sitter/tree-sitter-bash
- Copyright (c) 2017 Max Brunsfeld

### tree-sitter-c 0.24.2

- License: `MIT`
- Repository: https://github.com/tree-sitter/tree-sitter-c
- Copyright (c) 2014 Max Brunsfeld

### tree-sitter-cpp 0.23.4

- License: `MIT`
- Repository: https://github.com/tree-sitter/tree-sitter-cpp

### tree-sitter-elisp 1.6.1

- License: `MIT`
- Repository: https://github.com/Wilfred/tree-sitter-elisp
- Copyright (c) 2021 Wilfred Hughes

### tree-sitter-java 0.23.5

- License: `MIT`
- Repository: https://github.com/tree-sitter/tree-sitter-java

### tree-sitter-language 0.1.7

- License: `MIT`
- Repository: https://github.com/tree-sitter/tree-sitter
- Copyright (c) 2018 Max Brunsfeld

### tree-sitter-python 0.25.0

- License: `MIT`
- Repository: https://github.com/tree-sitter/tree-sitter-python
- Copyright (c) 2016 Max Brunsfeld

### tree-sitter-rust 0.23.3

- License: `MIT`
- Repository: https://github.com/tree-sitter/tree-sitter-rust

### tree-sitter-systemverilog 0.4.0

- License: `MIT`
- Repository: https://github.com/gmlarumbe/tree-sitter-systemverilog
- Copyright (c) 2024-2025 Gonzalo M. Larumbe

### ts-parser-perl 1.2.1

- License: `MIT`
- Repository: https://github.com/tree-sitter-perl/tree-sitter-perl
- Copyright 2025 Avishai "Veesh" Goldman

### ttf-parser 0.25.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/harfbuzz/ttf-parser
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2018 Yevhenii Reizner

### type-map 0.5.1

- License: `MIT/Apache-2.0`
- Repository: https://github.com/kardeiz/type-map
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2022 Jacob Brown

### typenum 1.20.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/paholg/typenum
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 Paho Lurie-Gregg

### uds_windows 1.2.1

- License: `MIT`
- Repository: https://github.com/haraldh/rust_uds_windows
- Copyright (c) Microsoft Corporation. All rights reserved.

### unicode-ident 1.0.24

- License: `(MIT OR Apache-2.0) AND Unicode-3.0`
- Repository: https://github.com/dtolnay/unicode-ident
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- COPYRIGHT AND PERMISSION NOTICE

### unicode-segmentation 1.13.3

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/unicode-rs/unicode-segmentation
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2015 The Rust Project Developers

### unicode-width 0.1.14

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/unicode-rs/unicode-width
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2015 The Rust Project Developers

### unicode-width 0.2.2

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/unicode-rs/unicode-width
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2015 The Rust Project Developers

### unicode-xid 0.2.6

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/unicode-rs/unicode-xid
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2015 The Rust Project Developers

### url 2.5.8

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/servo/rust-url
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2013-2025 The rust-url developers

### utf8_iter 1.0.4

- License: `Apache-2.0 OR MIT`
- Repository: https://github.com/hsivonen/utf8_iter
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright Mozilla Foundation

### version_check 0.9.5

- License: `MIT/Apache-2.0`
- Repository: https://github.com/SergioBenitez/version_check
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2017-2018 Sergio Benitez

### walkdir 2.5.0

- License: `Unlicense/MIT`
- Repository: https://github.com/BurntSushi/walkdir
- Copyright (c) 2015 Andrew Gallant

### wasi 0.11.1+wasi-snapshot-preview1

- License: `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT`
- Repository: https://github.com/bytecodealliance/wasi
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### wasip2 1.0.4+wasi-0.2.12

- License: `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT`
- Repository: https://github.com/bytecodealliance/wasi-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### wasm-bindgen 0.2.126

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/wasm-bindgen/wasm-bindgen
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 Alex Crichton

### wasm-bindgen-futures 0.4.76

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/futures
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 Alex Crichton

### wasm-bindgen-macro 0.2.126

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/macro
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 Alex Crichton

### wasm-bindgen-macro-support 0.2.126

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/macro-support
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 Alex Crichton

### wasm-bindgen-shared 0.2.126

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/shared
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 Alex Crichton

### wasmtime-internal-core 46.0.1

- License: `Apache-2.0 WITH LLVM-exception`

### wasmtime-internal-jit-icache-coherence 46.0.1

- License: `Apache-2.0 WITH LLVM-exception`
- Repository: https://github.com/bytecodealliance/wasmtime

### wayland-backend 0.3.15

- License: `MIT`
- Repository: https://github.com/smithay/wayland-rs
- Copyright (c) 2015 Elinor Berger

### wayland-client 0.31.14

- License: `MIT`
- Repository: https://github.com/smithay/wayland-rs
- Copyright (c) 2015 Elinor Berger

### wayland-csd-frame 0.3.0

- License: `MIT`
- Repository: https://github.com/rust-windowing/wayland-csd-frame
- Copyright (c) 2023 Kirill Chibisov

### wayland-cursor 0.31.14

- License: `MIT`
- Repository: https://github.com/smithay/wayland-rs
- Copyright (c) 2015 Elinor Berger

### wayland-protocols 0.32.13

- License: `MIT`
- Repository: https://github.com/smithay/wayland-rs
- Copyright (c) 2015 Elinor Berger

### wayland-protocols-experimental 20250721.0.1

- License: `MIT`
- Repository: https://github.com/smithay/wayland-rs
- Copyright (c) 2015 Elinor Berger

### wayland-protocols-misc 0.3.12

- License: `MIT`
- Repository: https://github.com/smithay/wayland-rs
- Copyright (c) 2015 Elinor Berger

### wayland-protocols-plasma 0.3.12

- License: `MIT`
- Repository: https://github.com/smithay/wayland-rs
- Copyright (c) 2015 Elinor Berger

### wayland-protocols-wlr 0.3.12

- License: `MIT`
- Repository: https://github.com/smithay/wayland-rs
- Copyright (c) 2015 Elinor Berger

### wayland-scanner 0.31.10

- License: `MIT`
- Repository: https://github.com/smithay/wayland-rs
- Copyright (c) 2015 Elinor Berger

### wayland-sys 0.31.11

- License: `MIT`
- Repository: https://github.com/smithay/wayland-rs
- Copyright (c) 2015 Elinor Berger

### web-sys 0.3.103

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/web-sys
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2014 Alex Crichton

### web-time 1.1.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/daxpedda/web-time
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2023 dAxpeDDa

### webbrowser 1.2.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/amodm/webbrowser-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2015-2022 Amod Malviya

### wgpu 22.1.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/gfx-rs/wgpu
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2021 The gfx-rs developers

### wgpu-core 22.1.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/gfx-rs/wgpu
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2021 The gfx-rs developers

### wgpu-hal 22.0.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/gfx-rs/wgpu
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2021 The gfx-rs developers

### wgpu-types 22.0.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/gfx-rs/wgpu
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2021 The gfx-rs developers

### widestring 1.2.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/VoidStarKat/widestring-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### winapi 0.3.9

- License: `MIT/Apache-2.0`
- Repository: https://github.com/retep998/winapi-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2015-2018 The winapi-rs Developers

### winapi-i686-pc-windows-gnu 0.4.0

- License: `MIT/Apache-2.0`
- Repository: https://github.com/retep998/winapi-rs

### winapi-util 0.1.11

- License: `Unlicense OR MIT`
- Repository: https://github.com/BurntSushi/winapi-util
- Copyright (c) 2017 Andrew Gallant

### winapi-x86_64-pc-windows-gnu 0.4.0

- License: `MIT/Apache-2.0`
- Repository: https://github.com/retep998/winapi-rs

### windows 0.52.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/microsoft/windows-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Microsoft Corporation.

### windows 0.58.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/microsoft/windows-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Microsoft Corporation.

### windows-core 0.52.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/microsoft/windows-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Microsoft Corporation.

### windows-core 0.58.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/microsoft/windows-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Microsoft Corporation.

### windows-implement 0.58.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/microsoft/windows-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Microsoft Corporation.

### windows-interface 0.58.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/microsoft/windows-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Microsoft Corporation.

### windows-link 0.2.1

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/microsoft/windows-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Microsoft Corporation.

### windows-result 0.2.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/microsoft/windows-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Microsoft Corporation.

### windows-strings 0.1.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/microsoft/windows-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Microsoft Corporation.

### windows-sys 0.52.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/microsoft/windows-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Microsoft Corporation.

### windows-sys 0.59.0

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/microsoft/windows-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Microsoft Corporation.

### windows-sys 0.61.2

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/microsoft/windows-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Microsoft Corporation.

### windows-targets 0.52.6

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/microsoft/windows-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Microsoft Corporation.

### windows_aarch64_gnullvm 0.52.6

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/microsoft/windows-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Microsoft Corporation.

### windows_aarch64_msvc 0.52.6

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/microsoft/windows-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Microsoft Corporation.

### windows_i686_gnu 0.52.6

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/microsoft/windows-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Microsoft Corporation.

### windows_i686_gnullvm 0.52.6

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/microsoft/windows-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Microsoft Corporation.

### windows_i686_msvc 0.52.6

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/microsoft/windows-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Microsoft Corporation.

### windows_x86_64_gnu 0.52.6

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/microsoft/windows-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Microsoft Corporation.

### windows_x86_64_gnullvm 0.52.6

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/microsoft/windows-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Microsoft Corporation.

### windows_x86_64_msvc 0.52.6

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/microsoft/windows-rs
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) Microsoft Corporation.

### winit 0.30.13

- License: `Apache-2.0`
- Repository: https://github.com/rust-windowing/winit
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### winnow 1.0.4

- License: `MIT`
- Repository: https://github.com/winnow-rs/winnow

### wit-bindgen 0.57.1

- License: `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT`
- Repository: https://github.com/bytecodealliance/wit-bindgen
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works

### writeable 0.6.3

- License: `Unicode-3.0`
- Repository: https://github.com/unicode-org/icu4x
- COPYRIGHT AND PERMISSION NOTICE
- Copyright © 2020-2024 Unicode, Inc.

### x11-dl 2.21.0

- License: `MIT`
- Repository: https://github.com/AltF02/x11-rs.git

### x11rb 0.13.2

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/psychon/x11rb
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright 2019 x11rb Contributers

### x11rb-protocol 0.13.2

- License: `MIT OR Apache-2.0`
- Repository: https://github.com/psychon/x11rb
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright 2019 x11rb Contributers

### xcursor 0.3.10

- License: `MIT`
- Repository: https://github.com/esposm03/xcursor-rs
- Copyright (c) 2020 Samuele Esposito

### xdg-home 1.3.0

- License: `MIT`
- Repository: https://github.com/zeenix/xdg-home

### xkbcommon-dl 0.4.2

- License: `MIT`
- Repository: https://github.com/rust-windowing/xkbcommon-dl
- Copyright (c) 2023 Kirill Chibisov

### xkeysym 0.2.1

- License: `MIT OR Apache-2.0 OR Zlib`
- Repository: https://github.com/notgull/xkeysym
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright (c) 2022-2023 John Nunley

### xml-rs 0.8.28

- License: `MIT`
- Repository: https://github.com/kornelski/xml-rs
- Copyright (c) 2014 Vladimir Matveev

### yoke 0.8.3

- License: `Unicode-3.0`
- Repository: https://github.com/unicode-org/icu4x
- COPYRIGHT AND PERMISSION NOTICE
- Copyright © 2020-2024 Unicode, Inc.

### yoke-derive 0.8.2

- License: `Unicode-3.0`
- Repository: https://github.com/unicode-org/icu4x
- COPYRIGHT AND PERMISSION NOTICE
- Copyright © 2020-2024 Unicode, Inc.

### zbus 4.4.0

- License: `MIT`
- Repository: https://github.com/dbus2/zbus/
- Copyright (c) 2024 Zeeshan Ali Khan & zbus contributors

### zbus-lockstep 0.4.4

- License: `MIT`
- Repository: https://github.com/luukvanderduim/zbus-lockstep

### zbus-lockstep-macros 0.4.4

- License: `MIT`
- Repository: https://github.com/luukvanderduim/zbus-lockstep

### zbus_macros 4.4.0

- License: `MIT`
- Repository: https://github.com/dbus2/zbus/
- Copyright (c) 2024 Zeeshan Ali Khan & zbus contributors

### zbus_names 3.0.0

- License: `MIT`
- Repository: https://github.com/dbus2/zbus/

### zbus_xml 4.0.0

- License: `MIT`
- Repository: https://github.com/dbus2/zbus/

### zerocopy 0.8.54

- License: `BSD-2-Clause OR Apache-2.0 OR MIT`
- Repository: https://github.com/google/zerocopy
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright 2019 The Fuchsia Authors.

### zerocopy-derive 0.8.54

- License: `BSD-2-Clause OR Apache-2.0 OR MIT`
- Repository: https://github.com/google/zerocopy
- copyright notice that is included in or attached to the work
- copyright license to reproduce, prepare Derivative Works of,
- (c) You must retain, in the Source form of any Derivative Works
- Copyright 2019 The Fuchsia Authors.

### zerofrom 0.1.8

- License: `Unicode-3.0`
- Repository: https://github.com/unicode-org/icu4x
- COPYRIGHT AND PERMISSION NOTICE
- Copyright © 2020-2024 Unicode, Inc.

### zerofrom-derive 0.1.7

- License: `Unicode-3.0`
- Repository: https://github.com/unicode-org/icu4x
- COPYRIGHT AND PERMISSION NOTICE
- Copyright © 2020-2024 Unicode, Inc.

### zerotrie 0.2.4

- License: `Unicode-3.0`
- Repository: https://github.com/unicode-org/icu4x
- COPYRIGHT AND PERMISSION NOTICE
- Copyright © 2020-2024 Unicode, Inc.

### zerovec 0.11.6

- License: `Unicode-3.0`
- Repository: https://github.com/unicode-org/icu4x
- COPYRIGHT AND PERMISSION NOTICE
- Copyright © 2020-2024 Unicode, Inc.

### zerovec-derive 0.11.3

- License: `Unicode-3.0`
- Repository: https://github.com/unicode-org/icu4x
- COPYRIGHT AND PERMISSION NOTICE
- Copyright © 2020-2024 Unicode, Inc.

### zmij 1.0.23

- License: `MIT`
- Repository: https://github.com/dtolnay/zmij

### zvariant 4.2.0

- License: `MIT`
- Repository: https://github.com/dbus2/zbus/
- Copyright (c) 2024 Zeeshan Ali Khan & zbus contributors

### zvariant_derive 4.2.0

- License: `MIT`
- Repository: https://github.com/dbus2/zbus/
- Copyright (c) 2024 Zeeshan Ali Khan & zbus contributors

### zvariant_utils 2.1.0

- License: `MIT`
- Repository: https://github.com/dbus2/zbus/

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

