# Third-party packages

Cargo.lock fixes the versions used by this build. License expressions below are package metadata, not a replacement for the original license text.

## Bundled icons

`assets/icons` embeds these files in the binary. `scripts/build_file_icons.py` regenerates them.

| Source | Files | License |
| --- | --- | --- |
| [Seti UI](https://github.com/jesseweed/seti-ui) — Copyright (c) 2014 Jesse Weed | `assets/icons/files/*.svg` except `markdown.svg` (re-framed viewBox) | MIT |
| [Markdown Mark](https://github.com/dcurtis/markdown-mark) — Dustin Curtis | `assets/icons/files/markdown.svg` | CC0-1.0 (public domain) |
| [Lucide](https://lucide.dev) via gpui-kit-assets 0.7.0 — Copyright (c) Lucide Icons and Contributors; Feather-derived icons Copyright (c) 2013-present Cole Bemis | `assets/icons/ui/*.svg` (stroke width 1.75) | ISC; Feather-derived icons MIT |

### Seti UI license

```
The MIT License (MIT)

Copyright (c) 2014 Jesse Weed

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
```

### Lucide license

As shipped in gpui-kit-assets 0.7.0 (`LICENSE-LUCIDE`):

```
ISC License

Copyright (c) 2026 Lucide Icons and Contributors

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.

---

The following Lucide icons are derived from the Feather project:

airplay, alert-circle, alert-octagon, alert-triangle, aperture, arrow-down-circle, arrow-down-left, arrow-down-right, arrow-down, arrow-left-circle, arrow-left, arrow-right-circle, arrow-right, arrow-up-circle, arrow-up-left, arrow-up-right, arrow-up, at-sign, calendar, cast, check, chevron-down, chevron-left, chevron-right, chevron-up, chevrons-down, chevrons-left, chevrons-right, chevrons-up, circle, clipboard, clock, code, columns, command, compass, corner-down-left, corner-down-right, corner-left-down, corner-left-up, corner-right-down, corner-right-up, corner-up-left, corner-up-right, crosshair, database, divide-circle, divide-square, dollar-sign, download, external-link, feather, frown, hash, headphones, help-circle, info, italic, key, layout, life-buoy, link-2, link, loader, lock, log-in, log-out, maximize, meh, minimize, minimize-2, minus-circle, minus-square, minus, monitor, moon, more-horizontal, more-vertical, move, music, navigation-2, navigation, octagon, pause-circle, percent, plus-circle, plus-square, plus, power, radio, rss, search, server, share, shopping-bag, sidebar, smartphone, smile, square, table-2, tablet, target, terminal, trash-2, trash, triangle, tv, type, upload, x-circle, x-octagon, x-square, x, zoom-in, zoom-out

The MIT License (MIT) (for the icons listed above)

Copyright (c) 2013-present Cole Bemis

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
```

## Packages

| Package | Version | License |
| --- | --- | --- |
| [accesskit](https://github.com/AccessKit/accesskit) | 0.24.1 | MIT OR Apache-2.0 |
| [accesskit_consumer](https://github.com/AccessKit/accesskit) | 0.38.0 | MIT OR Apache-2.0 |
| [accesskit_windows](https://github.com/AccessKit/accesskit) | 0.34.0 | MIT OR Apache-2.0 |
| [adler2](https://github.com/oyvindln/adler2) | 2.0.1 | 0BSD OR MIT OR Apache-2.0 |
| [aho-corasick](https://github.com/BurntSushi/aho-corasick) | 1.1.5 | Unlicense OR MIT |
| [alacritty_terminal](https://github.com/alacritty/alacritty) | 0.26.0 | Apache-2.0 |
| [aligned](https://github.com/rust-embedded-community/aligned) | 0.4.3 | MIT OR Apache-2.0 |
| [aligned-vec](https://github.com/sarah-ek/aligned-vec/) | 0.6.4 | MIT |
| [allocator-api2](https://github.com/zakarumych/allocator-api2) | 0.2.21 | MIT OR Apache-2.0 |
| [annotate-snippets](https://github.com/rust-lang/annotate-snippets-rs) | 0.12.16 | MIT OR Apache-2.0 |
| [anstream](https://github.com/rust-cli/anstyle.git) | 1.0.0 | MIT OR Apache-2.0 |
| [anstyle](https://github.com/rust-cli/anstyle.git) | 1.0.14 | MIT OR Apache-2.0 |
| [anstyle-parse](https://github.com/rust-cli/anstyle.git) | 1.0.0 | MIT OR Apache-2.0 |
| [anstyle-query](https://github.com/rust-cli/anstyle.git) | 1.1.5 | MIT OR Apache-2.0 |
| [anstyle-wincon](https://github.com/rust-cli/anstyle.git) | 3.0.11 | MIT OR Apache-2.0 |
| [anyhow](https://github.com/dtolnay/anyhow) | 1.0.104 | MIT OR Apache-2.0 |
| [arc-swap](https://github.com/vorner/arc-swap) | 1.9.2 | MIT OR Apache-2.0 |
| [arg_enum_proc_macro](https://github.com/lu-zero/arg_enum_proc_macro) | 0.3.4 | MIT |
| [arraydeque](https://github.com/andylokandy/arraydeque) | 0.5.1 | MIT/Apache-2.0 |
| [arrayref](https://github.com/droundy/arrayref) | 0.3.9 | BSD-2-Clause |
| [arrayvec](https://github.com/bluss/arrayvec) | 0.7.8 | MIT OR Apache-2.0 |
| [as-slice](https://github.com/japaric/as-slice) | 0.2.1 | MIT OR Apache-2.0 |
| [async-channel](https://github.com/smol-rs/async-channel) | 2.5.0 | Apache-2.0 OR MIT |
| [async-compression](https://github.com/Nullus157/async-compression) | 0.4.48 | MIT OR Apache-2.0 |
| [async-executor](https://github.com/smol-rs/async-executor) | 1.14.0 | Apache-2.0 OR MIT |
| [async-fs](https://github.com/smol-rs/async-fs) | 2.2.0 | Apache-2.0 OR MIT |
| [async-io](https://github.com/smol-rs/async-io) | 2.6.0 | Apache-2.0 OR MIT |
| [async-lock](https://github.com/smol-rs/async-lock) | 3.4.2 | Apache-2.0 OR MIT |
| [async-net](https://github.com/smol-rs/async-net) | 2.0.0 | Apache-2.0 OR MIT |
| [async-process](https://github.com/smol-rs/async-process) | 2.5.0 | Apache-2.0 OR MIT |
| [async-task](https://github.com/smol-rs/async-task) | 4.7.1 | Apache-2.0 OR MIT |
| [atomic](https://github.com/Amanieu/atomic-rs) | 0.5.3 | Apache-2.0/MIT |
| [atomic-waker](https://github.com/smol-rs/atomic-waker) | 1.1.2 | Apache-2.0 OR MIT |
| [autocfg](https://github.com/cuviper/autocfg) | 1.5.1 | Apache-2.0 OR MIT |
| [av-scenechange](https://github.com/rust-av/av-scenechange) | 0.14.1 | MIT |
| [av1-grain](https://github.com/rust-av/av1-grain) | 0.2.5 | BSD-2-Clause |
| [avif-serialize](https://github.com/kornelski/avif-serialize) | 0.8.9 | BSD-3-Clause |
| [backtrace](https://github.com/rust-lang/backtrace-rs) | 0.3.76 | MIT OR Apache-2.0 |
| [base62](https://github.com/fbernier/base62) | 2.2.6 | MIT |
| [base64](https://github.com/marshallpierce/rust-base64) | 0.22.1 | MIT OR Apache-2.0 |
| [base64](https://github.com/marshallpierce/rust-base64) | 0.23.1 | MIT OR Apache-2.0 |
| [bit_field](https://github.com/phil-opp/rust-bit-field) | 0.10.3 | Apache-2.0/MIT |
| [bitflags](https://github.com/bitflags/bitflags) | 1.3.2 | MIT/Apache-2.0 |
| [bitflags](https://github.com/bitflags/bitflags) | 2.13.2 | MIT OR Apache-2.0 |
| [bitstream-io](https://github.com/tuffy/bitstream-io) | 4.10.0 | MIT/Apache-2.0 |
| [blake3](https://github.com/BLAKE3-team/BLAKE3) | 1.8.7 | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception |
| [block-buffer](https://github.com/RustCrypto/utils) | 0.12.1 | MIT OR Apache-2.0 |
| [blocking](https://github.com/smol-rs/blocking) | 1.7.0 | Apache-2.0 OR MIT |
| [borsh](https://github.com/near/borsh-rs) | 1.8.1 | MIT OR Apache-2.0 |
| [bstr](https://github.com/BurntSushi/bstr) | 1.13.1 | MIT OR Apache-2.0 |
| [built](https://github.com/lukaslueg/built) | 0.8.1 | MIT |
| [bumpalo](https://github.com/fitzgen/bumpalo) | 3.20.3 | MIT OR Apache-2.0 |
| [bytemuck](https://github.com/Lokathor/bytemuck) | 1.25.2 | Zlib OR Apache-2.0 OR MIT |
| [bytemuck_derive](https://github.com/Lokathor/bytemuck) | 1.12.1 | Zlib OR Apache-2.0 OR MIT |
| [byteorder](https://github.com/BurntSushi/byteorder) | 1.5.0 | Unlicense OR MIT |
| [byteorder-lite](https://github.com/image-rs/byteorder-lite) | 0.1.0 | Unlicense OR MIT |
| [bytes](https://github.com/tokio-rs/bytes) | 1.12.1 | MIT |
| [bzip2](https://github.com/trifectatechfoundation/bzip2-rs) | 0.6.1 | MIT OR Apache-2.0 |
| [cc](https://github.com/rust-lang/cc-rs) | 1.5.1 | MIT OR Apache-2.0 |
| [cfg-if](https://github.com/rust-lang/cfg-if) | 1.0.5 | MIT OR Apache-2.0 |
| [cfg_aliases](https://github.com/katharostech/cfg_aliases) | 0.2.2 | MIT |
| [chrono](https://github.com/chronotope/chrono) | 0.4.45 | MIT OR Apache-2.0 |
| [color_quant](https://github.com/image-rs/color_quant.git) | 1.1.0 | MIT |
| [colorchoice](https://github.com/rust-cli/anstyle.git) | 1.0.5 | MIT OR Apache-2.0 |
| [compression-codecs](https://github.com/Nullus157/async-compression) | 0.4.43 | MIT OR Apache-2.0 |
| [compression-core](https://github.com/Nullus157/async-compression) | 0.4.33 | MIT OR Apache-2.0 |
| [concurrent-queue](https://github.com/smol-rs/concurrent-queue) | 2.5.0 | Apache-2.0 OR MIT |
| [const-oid](https://github.com/RustCrypto/formats) | 0.10.2 | Apache-2.0 OR MIT |
| [constant_time_eq](https://github.com/cesarb/constant_time_eq) | 0.4.2 | CC0-1.0 OR MIT-0 OR Apache-2.0 |
| [convert_case](https://github.com/rutrum/convert-case) | 0.10.0 | MIT |
| [core_detect](https://github.com/thomcc/core_detect) | 1.0.0 | MIT/Apache-2.0 |
| [core_maths](https://github.com/robertbastian/core_maths) | 0.1.1 | MIT |
| [cpufeatures](https://github.com/RustCrypto/utils) | 0.3.1 | MIT OR Apache-2.0 |
| [crc32fast](https://github.com/srijs/rust-crc32fast) | 1.5.2 | MIT OR Apache-2.0 |
| [crossbeam-deque](https://github.com/crossbeam-rs/crossbeam) | 0.8.8 | MIT OR Apache-2.0 |
| [crossbeam-epoch](https://github.com/crossbeam-rs/crossbeam) | 0.9.21 | MIT OR Apache-2.0 |
| [crossbeam-queue](https://github.com/crossbeam-rs/crossbeam) | 0.3.14 | MIT OR Apache-2.0 |
| [crossbeam-utils](https://github.com/crossbeam-rs/crossbeam) | 0.8.23 | MIT OR Apache-2.0 |
| [crypto-common](https://github.com/RustCrypto/traits) | 0.2.2 | MIT OR Apache-2.0 |
| [ctor](https://github.com/mmastrac/linktime) | 1.0.13 | Apache-2.0 OR MIT |
| [cursor-icon](https://github.com/rust-windowing/cursor-icon) | 1.2.0 | MIT OR Apache-2.0 OR Zlib |
| [data-url](https://github.com/servo/rust-url) | 0.3.2 | MIT OR Apache-2.0 |
| [defmt](https://github.com/knurling-rs/defmt) | 1.1.1 | MIT OR Apache-2.0 |
| [defmt-macros](https://github.com/knurling-rs/defmt) | 1.1.1 | MIT OR Apache-2.0 |
| [defmt-parser](https://github.com/knurling-rs/defmt) | 1.0.0 | MIT OR Apache-2.0 |
| [derive_more](https://github.com/JelteF/derive_more) | 2.1.1 | MIT |
| [derive_more-impl](https://github.com/JelteF/derive_more) | 2.1.1 | MIT |
| [digest](https://github.com/RustCrypto/traits) | 0.11.3 | MIT OR Apache-2.0 |
| [dirs](https://github.com/soc/dirs-rs) | 6.0.0 | MIT OR Apache-2.0 |
| [dirs-sys](https://github.com/dirs-dev/dirs-sys-rs) | 0.5.0 | MIT OR Apache-2.0 |
| [displaydoc](https://github.com/yaahc/displaydoc) | 0.2.7 | MIT OR Apache-2.0 |
| [dunce](https://gitlab.com/kornelski/dunce) | 1.0.5 | CC0-1.0 OR MIT-0 OR Apache-2.0 |
| [dyn-clone](https://github.com/dtolnay/dyn-clone) | 1.0.20 | MIT OR Apache-2.0 |
| [either](https://github.com/rayon-rs/either) | 1.18.0 | MIT OR Apache-2.0 |
| [embed-resource](https://github.com/nabijaczleweli/rust-embed-resource) | 3.0.11 | MIT |
| [encoding_rs](https://github.com/hsivonen/encoding_rs) | 0.8.42 | (Apache-2.0 OR MIT) AND BSD-3-Clause |
| [encoding_rs_io](https://github.com/BurntSushi/encoding_rs_io) | 0.1.8 | MIT OR Apache-2.0 |
| [enum-iterator](https://github.com/stephaneyfx/enum-iterator.git) | 2.3.0 | 0BSD |
| [enum-iterator-derive](https://github.com/stephaneyfx/enum-iterator.git) | 1.5.0 | 0BSD |
| [enumn](https://github.com/dtolnay/enumn) | 0.1.14 | MIT OR Apache-2.0 |
| [env_filter](https://github.com/rust-cli/env_logger) | 2.0.0 | MIT OR Apache-2.0 |
| [env_logger](https://github.com/rust-cli/env_logger) | 0.11.11 | MIT OR Apache-2.0 |
| [equator](https://github.com/sarah-ek/equator/) | 0.4.2 | MIT |
| [equator-macro](https://github.com/sarah-ek/equator/) | 0.4.2 | MIT |
| [equivalent](https://github.com/indexmap-rs/equivalent) | 1.0.2 | Apache-2.0 OR MIT |
| [erased-serde](https://github.com/dtolnay/erased-serde) | 0.4.10 | MIT OR Apache-2.0 |
| [errno](https://github.com/lambda-fairy/rust-errno) | 0.3.14 | MIT OR Apache-2.0 |
| [etagere](https://github.com/nical/etagere) | 0.2.15 | MIT/Apache-2.0 |
| [euclid](https://github.com/servo/euclid) | 0.22.14 | MIT OR Apache-2.0 |
| [event-listener](https://github.com/smol-rs/event-listener) | 5.4.2 | Apache-2.0 OR MIT |
| [event-listener-strategy](https://github.com/smol-rs/event-listener-strategy) | 0.5.4 | Apache-2.0 OR MIT |
| [exr](https://github.com/johannesvollmer/exrs) | 1.74.2 | BSD-3-Clause |
| [fastrand](https://github.com/smol-rs/fastrand) | 2.5.0 | Apache-2.0 OR MIT |
| [fax](https://github.com/pdf-rs/fax) | 0.2.7 | MIT |
| [fdeflate](https://github.com/image-rs/fdeflate) | 0.3.7 | MIT OR Apache-2.0 |
| [filetime](https://github.com/alexcrichton/filetime) | 0.2.29 | MIT/Apache-2.0 |
| [find-msvc-tools](https://github.com/rust-lang/cc-rs) | 0.1.14 | MIT OR Apache-2.0 |
| [fixedbitset](https://github.com/petgraph/fixedbitset) | 0.5.7 | MIT OR Apache-2.0 |
| [flate2](https://github.com/rust-lang/flate2-rs) | 1.1.10 | MIT OR Apache-2.0 |
| [float-cmp](https://github.com/mikedilger/float-cmp) | 0.9.0 | MIT |
| [float_next_after](https://gitlab.com/bronsonbdevost/next_afterf) | 1.0.0 | MIT |
| [fluent-uri](https://github.com/yescallop/fluent-uri-rs) | 0.1.4 | MIT |
| [flume](https://github.com/zesterer/flume) | 0.12.0 | Apache-2.0/MIT |
| [foldhash](https://github.com/orlp/foldhash) | 0.2.0 | Zlib |
| [fontdb](https://github.com/RazrFalcon/fontdb) | 0.23.0 | MIT |
| [form_urlencoded](https://github.com/servo/rust-url) | 1.2.2 | MIT OR Apache-2.0 |
| [futf](https://github.com/servo/futf) | 0.1.5 | MIT / Apache-2.0 |
| [futures](https://github.com/rust-lang/futures-rs) | 0.3.34 | MIT OR Apache-2.0 |
| [futures-channel](https://github.com/rust-lang/futures-rs) | 0.3.34 | MIT OR Apache-2.0 |
| [futures-concurrency](https://github.com/yoshuawuyts/futures-concurrency) | 7.7.1 | MIT OR Apache-2.0 |
| [futures-core](https://github.com/rust-lang/futures-rs) | 0.3.34 | MIT OR Apache-2.0 |
| [futures-executor](https://github.com/rust-lang/futures-rs) | 0.3.34 | MIT OR Apache-2.0 |
| [futures-io](https://github.com/rust-lang/futures-rs) | 0.3.34 | MIT OR Apache-2.0 |
| [futures-lite](https://github.com/smol-rs/futures-lite) | 2.6.1 | Apache-2.0 OR MIT |
| [futures-macro](https://github.com/rust-lang/futures-rs) | 0.3.34 | MIT OR Apache-2.0 |
| [futures-sink](https://github.com/rust-lang/futures-rs) | 0.3.34 | MIT OR Apache-2.0 |
| [futures-task](https://github.com/rust-lang/futures-rs) | 0.3.34 | MIT OR Apache-2.0 |
| [futures-util](https://github.com/rust-lang/futures-rs) | 0.3.34 | MIT OR Apache-2.0 |
| [getrandom](https://github.com/rust-random/getrandom) | 0.2.17 | MIT OR Apache-2.0 |
| [getrandom](https://github.com/rust-random/getrandom) | 0.3.4 | MIT OR Apache-2.0 |
| [getrandom](https://github.com/rust-random/getrandom) | 0.4.3 | MIT OR Apache-2.0 |
| [gif](https://github.com/image-rs/image-gif) | 0.13.3 | MIT OR Apache-2.0 |
| [gif](https://github.com/image-rs/image-gif) | 0.14.2 | MIT OR Apache-2.0 |
| [glob](https://github.com/rust-lang/glob) | 0.3.4 | MIT OR Apache-2.0 |
| [globset](https://github.com/BurntSushi/ripgrep/tree/master/crates/globset) | 0.4.20 | Unlicense OR MIT |
| [globwalk](https://github.com/gilnaa/globwalk) | 0.8.1 | MIT |
| [gpui-base](https://github.com/longbridge/gpui-kit) | 0.7.0 | Apache-2.0 |
| [gpui-component](https://github.com/longbridge/gpui-kit) | 0.7.0 | Apache-2.0 |
| [gpui-component-macros](https://crates.io/crates/gpui-component-macros) | 0.7.0 | Apache-2.0 |
| [gpui-kit](https://github.com/longbridge/gpui-kit) | 0.7.0 | Apache-2.0 |
| [gpui-kit-assets](https://github.com/longbridge/gpui-kit) | 0.7.0 | Apache-2.0 |
| [gpui-pre](https://github.com/zed-industries/zed) | 0.3.7 | Apache-2.0 |
| [gpui-pre-collections](https://github.com/zed-industries/zed) | 0.3.7 | Apache-2.0 |
| [gpui-pre-derive-refineable](https://github.com/zed-industries/zed) | 0.3.7 | Apache-2.0 |
| [gpui-pre-http-client](https://github.com/zed-industries/zed) | 0.3.7 | Apache-2.0 |
| [gpui-pre-macros](https://github.com/zed-industries/zed) | 0.3.7 | Apache-2.0 |
| [gpui-pre-perf](https://github.com/zed-industries/zed) | 0.3.7 | Apache-2.0 |
| [gpui-pre-platform](https://github.com/zed-industries/zed) | 0.3.7 | Apache-2.0 |
| [gpui-pre-refineable](https://github.com/zed-industries/zed) | 0.3.7 | Apache-2.0 |
| [gpui-pre-scheduler](https://github.com/zed-industries/zed) | 0.3.7 | Apache-2.0 |
| [gpui-pre-shared-string](https://github.com/zed-industries/zed) | 0.3.7 | Apache-2.0 |
| [gpui-pre-sum-tree](https://github.com/zed-industries/zed) | 0.3.7 | Apache-2.0 |
| [gpui-pre-util](https://github.com/zed-industries/zed) | 0.3.7 | Apache-2.0 |
| [gpui-pre-util-macros](https://github.com/zed-industries/zed) | 0.3.7 | Apache-2.0 |
| [gpui-pre-windows](https://github.com/zed-industries/zed) | 0.3.7 | Apache-2.0 |
| [gpui-pre-zlog](https://github.com/zed-industries/zed) | 0.3.7 | Apache-2.0 |
| [gpui-pre-ztracing](https://github.com/zed-industries/zed) | 0.3.7 | Apache-2.0 |
| [gpui-pre-ztracing-macro](https://github.com/zed-industries/zed) | 0.3.7 | Apache-2.0 |
| [granit-parser](https://github.com/bourumir-wyngs/granit-parser) | 1.3.0 | MIT OR Apache-2.0 |
| [half](https://github.com/VoidStarKat/half-rs) | 2.7.1 | MIT OR Apache-2.0 |
| [hash32](https://github.com/japaric/hash32) | 0.3.1 | MIT OR Apache-2.0 |
| [hashbrown](https://github.com/rust-lang/hashbrown) | 0.16.1 | MIT OR Apache-2.0 |
| [hashbrown](https://github.com/rust-lang/hashbrown) | 0.17.1 | MIT OR Apache-2.0 |
| [heapless](https://github.com/rust-embedded/heapless) | 0.9.3 | MIT OR Apache-2.0 |
| [heck](https://github.com/withoutboats/heck) | 0.5.0 | MIT OR Apache-2.0 |
| [home](https://github.com/rust-lang/cargo) | 0.5.12 | MIT OR Apache-2.0 |
| [html5ever](https://github.com/servo/html5ever) | 0.27.0 | MIT OR Apache-2.0 |
| [http](https://github.com/hyperium/http) | 1.5.0 | MIT OR Apache-2.0 |
| [http-body](https://github.com/hyperium/http-body) | 1.1.0 | MIT |
| [hybrid-array](https://github.com/RustCrypto/hybrid-array) | 0.4.15 | MIT OR Apache-2.0 |
| [icu_collections](https://github.com/unicode-org/icu4x) | 2.3.0 | Unicode-3.0 |
| [icu_locale_core](https://github.com/unicode-org/icu4x) | 2.3.0 | Unicode-3.0 |
| [icu_normalizer](https://github.com/unicode-org/icu4x) | 2.3.0 | Unicode-3.0 |
| [icu_normalizer_data](https://github.com/unicode-org/icu4x) | 2.3.0 | Unicode-3.0 |
| [icu_properties](https://github.com/unicode-org/icu4x) | 2.3.0 | Unicode-3.0 |
| [icu_properties_data](https://github.com/unicode-org/icu4x) | 2.3.0 | Unicode-3.0 |
| [icu_provider](https://github.com/unicode-org/icu4x) | 2.3.1 | Unicode-3.0 |
| [idna](https://github.com/servo/rust-url/) | 1.1.0 | MIT OR Apache-2.0 |
| [idna_adapter](https://github.com/hsivonen/idna_adapter) | 1.2.2 | Apache-2.0 OR MIT |
| [ignore](https://github.com/BurntSushi/ripgrep/tree/master/crates/ignore) | 0.4.33 | Unlicense OR MIT |
| [image](https://github.com/image-rs/image) | 0.25.10 | MIT OR Apache-2.0 |
| [image-webp](https://github.com/image-rs/image-webp) | 0.2.4 | MIT OR Apache-2.0 |
| [imagesize](https://github.com/Roughsketch/imagesize) | 0.13.0 | MIT |
| [imagesize](https://github.com/Roughsketch/imagesize) | 0.14.0 | MIT |
| [imgref](https://github.com/kornelski/imgref) | 1.12.3 | CC0-1.0 OR Apache-2.0 |
| [indexmap](https://github.com/indexmap-rs/indexmap) | 2.14.2 | Apache-2.0 OR MIT |
| [instant](https://github.com/sebcrozet/instant) | 0.1.13 | BSD-3-Clause |
| [inventory](https://github.com/dtolnay/inventory) | 0.3.24 | MIT OR Apache-2.0 |
| [is_terminal_polyfill](https://github.com/polyfill-rs/is_terminal_polyfill) | 1.70.2 | MIT OR Apache-2.0 |
| [itertools](https://github.com/rust-itertools/itertools) | 0.11.0 | MIT OR Apache-2.0 |
| [itertools](https://github.com/rust-itertools/itertools) | 0.13.0 | MIT OR Apache-2.0 |
| [itertools](https://github.com/rust-itertools/itertools) | 0.14.0 | MIT OR Apache-2.0 |
| [itoa](https://github.com/dtolnay/itoa) | 1.0.18 | MIT OR Apache-2.0 |
| [jiff](https://github.com/BurntSushi/jiff) | 0.2.37 | Unlicense OR MIT |
| [jiff-core](https://github.com/BurntSushi/jiff) | 0.1.1 | Unlicense OR MIT |
| [jobserver](https://github.com/rust-lang/jobserver-rs) | 0.1.35 | MIT OR Apache-2.0 |
| [kurbo](https://github.com/linebender/kurbo) | 0.11.3 | Apache-2.0 OR MIT |
| [kurbo](https://github.com/linebender/kurbo) | 0.13.1 | Apache-2.0 OR MIT |
| [lazy_static](https://github.com/rust-lang-nursery/lazy-static.rs) | 1.5.0 | MIT OR Apache-2.0 |
| [lebe](https://github.com/johannesvollmer/lebe) | 0.5.3 | BSD-3-Clause |
| [libbz2-rs-sys](https://github.com/trifectatechfoundation/libbzip2-rs) | 0.2.5 | bzip2-1.0.6 |
| [libc](https://github.com/rust-lang/libc) | 0.2.189 | MIT OR Apache-2.0 |
| [libm](https://github.com/rust-lang/compiler-builtins) | 0.2.16 | MIT |
| [link-section](https://github.com/mmastrac/linktime) | 0.19.3 | Apache-2.0 OR MIT |
| [linktime-proc-macro](https://github.com/mmastrac/linktime) | 0.2.3 | Apache-2.0 OR MIT |
| [litemap](https://github.com/unicode-org/icu4x) | 0.8.3 | Unicode-3.0 |
| [lock_api](https://github.com/Amanieu/parking_lot) | 0.4.14 | MIT OR Apache-2.0 |
| [log](https://github.com/rust-lang/log) | 0.4.34 | MIT OR Apache-2.0 |
| [loop9](https://gitlab.com/kornelski/loop9.git) | 0.1.5 | MIT |
| [lsp-types](https://github.com/gluon-lang/lsp-types) | 0.97.0 | MIT |
| [lyon](https://github.com/nical/lyon) | 1.0.19 | MIT OR Apache-2.0 |
| [lyon_algorithms](https://github.com/nical/lyon) | 1.0.21 | MIT OR Apache-2.0 |
| [lyon_geom](https://github.com/nical/lyon) | 1.0.19 | MIT OR Apache-2.0 |
| [lyon_path](https://github.com/nical/lyon) | 1.0.19 | MIT OR Apache-2.0 |
| [lyon_tessellation](https://github.com/nical/lyon) | 1.0.22 | MIT OR Apache-2.0 |
| [mac](https://github.com/reem/rust-mac.git) | 0.1.1 | MIT/Apache-2.0 |
| [markdown](https://github.com/wooorm/markdown-rs) | 1.0.0 | MIT |
| [markup5ever](https://github.com/servo/html5ever) | 0.12.1 | MIT OR Apache-2.0 |
| [markup5ever_rcdom](https://github.com/servo/html5ever) | 0.3.0 | MIT OR Apache-2.0 |
| [maybe-rayon](https://github.com/shssoichiro/maybe-rayon) | 0.1.1 | MIT |
| [memchr](https://github.com/BurntSushi/memchr) | 2.8.3 | Unlicense OR MIT |
| [memmap2](https://github.com/RazrFalcon/memmap2-rs) | 0.9.11 | MIT OR Apache-2.0 |
| [mime](https://github.com/hyperium/mime) | 0.3.17 | MIT OR Apache-2.0 |
| [mime_guess](https://github.com/abonander/mime_guess) | 2.0.5 | MIT |
| [miniz_oxide](https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide) | 0.8.9 | MIT OR Zlib OR Apache-2.0 |
| [miniz_oxide](https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide) | 0.9.1 | MIT OR Zlib OR Apache-2.0 |
| [miow](https://github.com/yoshuawuyts/miow) | 0.6.1 | MIT OR Apache-2.0 |
| [moxcms](https://github.com/awxkee/moxcms.git) | 0.8.1 | BSD-3-Clause OR Apache-2.0 |
| [multiversion_no_op](https://github.com/hsivonen/multiversion_no_op) | 1.0.0 | Apache-2.0 OR MIT |
| [new_debug_unreachable](https://github.com/mbrubeck/rust-debug-unreachable) | 1.0.6 | MIT |
| [no_std_io2](https://github.com/wcampbell0x2a/no-std-io2) | 0.9.4 | Apache-2.0 OR MIT |
| [nohash-hasher](https://github.com/paritytech/nohash-hasher) | 0.2.0 | Apache-2.0 OR MIT |
| [nom](https://github.com/rust-bakery/nom) | 8.0.0 | MIT |
| [noop_proc_macro](https://github.com/lu-zero/noop_proc_macro) | 0.3.0 | MIT |
| [normpath](https://github.com/dylni/normpath) | 1.5.2 | MIT OR Apache-2.0 |
| [notify](https://github.com/notify-rs/notify.git) | 7.0.0 | CC0-1.0 |
| [notify](https://github.com/notify-rs/notify.git) | 8.2.0 | CC0-1.0 |
| [notify-types](https://github.com/notify-rs/notify.git) | 1.0.1 | MIT OR Apache-2.0 |
| [notify-types](https://github.com/notify-rs/notify.git) | 2.1.0 | MIT OR Apache-2.0 |
| [ntapi](https://github.com/MSxDOS/ntapi) | 0.4.3 | Apache-2.0 OR MIT |
| [nu-ansi-term](https://github.com/nushell/nu-ansi-term) | 0.50.3 | MIT |
| [num-bigint](https://github.com/rust-num/num-bigint) | 0.4.8 | MIT OR Apache-2.0 |
| [num-complex](https://github.com/rust-num/num-complex) | 0.4.6 | MIT OR Apache-2.0 |
| [num-derive](https://github.com/rust-num/num-derive) | 0.4.2 | MIT OR Apache-2.0 |
| [num-integer](https://github.com/rust-num/num-integer) | 0.1.47 | MIT OR Apache-2.0 |
| [num-rational](https://github.com/rust-num/num-rational) | 0.4.2 | MIT OR Apache-2.0 |
| [num-traits](https://github.com/rust-num/num-traits) | 0.2.19 | MIT OR Apache-2.0 |
| [num_cpus](https://github.com/seanmonstar/num_cpus) | 1.17.0 | MIT OR Apache-2.0 |
| [once_cell](https://github.com/matklad/once_cell) | 1.21.4 | MIT OR Apache-2.0 |
| [once_cell_polyfill](https://github.com/polyfill-rs/once_cell_polyfill) | 1.70.2 | MIT OR Apache-2.0 |
| [option-ext](https://github.com/soc/option-ext.git) | 0.2.0 | MPL-2.0 |
| [parking](https://github.com/smol-rs/parking) | 2.2.1 | Apache-2.0 OR MIT |
| [parking_lot](https://github.com/Amanieu/parking_lot) | 0.12.5 | MIT OR Apache-2.0 |
| [parking_lot_core](https://github.com/Amanieu/parking_lot) | 0.9.12 | MIT OR Apache-2.0 |
| [paste](https://github.com/dtolnay/paste) | 1.0.15 | MIT OR Apache-2.0 |
| [pastey](https://github.com/as1100k/pastey) | 0.1.1 | MIT OR Apache-2.0 |
| [percent-encoding](https://github.com/servo/rust-url/) | 2.3.2 | MIT OR Apache-2.0 |
| [phf](https://github.com/rust-phf/rust-phf) | 0.11.3 | MIT |
| [phf_codegen](https://github.com/rust-phf/rust-phf) | 0.11.3 | MIT |
| [phf_generator](https://github.com/rust-phf/rust-phf) | 0.11.3 | MIT |
| [phf_shared](https://github.com/rust-phf/rust-phf) | 0.11.3 | MIT |
| [pico-args](https://github.com/RazrFalcon/pico-args) | 0.5.0 | MIT |
| [pin-project](https://github.com/taiki-e/pin-project) | 1.1.13 | Apache-2.0 OR MIT |
| [pin-project-internal](https://github.com/taiki-e/pin-project) | 1.1.13 | Apache-2.0 OR MIT |
| [pin-project-lite](https://github.com/taiki-e/pin-project-lite) | 0.2.17 | Apache-2.0 OR MIT |
| [piper](https://github.com/smol-rs/piper) | 0.2.5 | MIT OR Apache-2.0 |
| [png](https://github.com/image-rs/image-png) | 0.17.16 | MIT OR Apache-2.0 |
| [png](https://github.com/image-rs/image-png) | 0.18.1 | MIT OR Apache-2.0 |
| [polling](https://github.com/smol-rs/polling) | 3.11.0 | Apache-2.0 OR MIT |
| [pollster](https://github.com/zesterer/pollster) | 0.2.5 | Apache-2.0/MIT |
| [pollster](https://github.com/zesterer/pollster) | 0.4.0 | Apache-2.0/MIT |
| [polycool](https://github.com/linebender/kurbo) | 0.4.0 | MIT OR Apache-2.0 |
| [postage](https://github.com/austinjones/postage-rs) | 0.5.0 | MIT |
| [potential_utf](https://github.com/unicode-org/icu4x) | 0.1.6 | Unicode-3.0 |
| [ppv-lite86](https://github.com/cryptocorrosion/cryptocorrosion) | 0.2.21 | MIT OR Apache-2.0 |
| [precomputed-hash](https://github.com/emilio/precomputed-hash) | 0.1.1 | MIT |
| [proc-macro-crate](https://github.com/bkchr/proc-macro-crate) | 3.5.0 | MIT OR Apache-2.0 |
| [proc-macro2](https://github.com/dtolnay/proc-macro2) | 1.0.107 | MIT OR Apache-2.0 |
| [profiling](https://github.com/aclysma/profiling) | 1.0.18 | MIT OR Apache-2.0 |
| [profiling-procmacros](https://github.com/aclysma/profiling) | 1.0.18 | MIT OR Apache-2.0 |
| [pulp](https://github.com/sarah-quinones/pulp/) | 0.22.3 | MIT |
| [pulp-wasm-simd-flag](https://github.com/sarah-quinones/pulp/) | 0.1.1 | MIT |
| [pxfm](https://github.com/awxkee/pxfm) | 0.1.30 | BSD-3-Clause OR Apache-2.0 |
| [qoi](https://github.com/aldanor/qoi-rust) | 0.4.1 | MIT/Apache-2.0 |
| [quick-error](http://github.com/tailhook/quick-error) | 2.0.1 | MIT/Apache-2.0 |
| [quote](https://github.com/dtolnay/quote) | 1.0.47 | MIT OR Apache-2.0 |
| [rand](https://github.com/rust-random/rand) | 0.8.8 | MIT OR Apache-2.0 |
| [rand](https://github.com/rust-random/rand) | 0.9.5 | MIT OR Apache-2.0 |
| [rand_chacha](https://github.com/rust-random/rand) | 0.3.1 | MIT OR Apache-2.0 |
| [rand_chacha](https://github.com/rust-random/rand) | 0.9.0 | MIT OR Apache-2.0 |
| [rand_core](https://github.com/rust-random/rand_core) | 0.10.1 | MIT OR Apache-2.0 |
| [rand_core](https://github.com/rust-random/rand) | 0.6.4 | MIT OR Apache-2.0 |
| [rand_core](https://github.com/rust-random/rand) | 0.9.5 | MIT OR Apache-2.0 |
| [rav1e](https://github.com/xiph/rav1e/) | 0.8.1 | BSD-2-Clause |
| [ravif](https://github.com/kornelski/cavif-rs) | 0.13.0 | BSD-3-Clause |
| [raw-cpuid](https://github.com/gz/rust-cpuid) | 11.6.0 | MIT |
| [raw-window-handle](https://github.com/rust-windowing/raw-window-handle) | 0.6.2 | MIT OR Apache-2.0 OR Zlib |
| [rayon](https://github.com/rayon-rs/rayon) | 1.12.0 | MIT OR Apache-2.0 |
| [rayon-core](https://github.com/rayon-rs/rayon) | 1.13.0 | MIT OR Apache-2.0 |
| [reborrow](https://github.com/sarah-ek/reborrow/) | 0.5.5 | MIT |
| [ref-cast](https://github.com/dtolnay/ref-cast) | 1.0.27 | MIT OR Apache-2.0 |
| [ref-cast-impl](https://github.com/dtolnay/ref-cast) | 1.0.27 | MIT OR Apache-2.0 |
| [regex](https://github.com/rust-lang/regex) | 1.13.1 | MIT OR Apache-2.0 |
| [regex-automata](https://github.com/rust-lang/regex) | 0.4.18 | MIT OR Apache-2.0 |
| [regex-syntax](https://github.com/rust-lang/regex) | 0.8.11 | MIT OR Apache-2.0 |
| [resvg](https://github.com/linebender/resvg) | 0.45.1 | Apache-2.0 OR MIT |
| [resvg](https://github.com/linebender/resvg) | 0.46.0 | Apache-2.0 OR MIT |
| [rgb](https://github.com/kornelski/rust-rgb) | 0.8.53 | MIT |
| [ropey](https://github.com/cessen/ropey) | 2.0.0-beta.1 | MIT OR Apache-2.0 |
| [roxmltree](https://github.com/RazrFalcon/roxmltree) | 0.20.0 | MIT OR Apache-2.0 |
| [roxmltree](https://github.com/RazrFalcon/roxmltree) | 0.21.1 | MIT OR Apache-2.0 |
| [rust-embed](https://pyrossh.dev/repos/rust-embed) | 8.12.0 | MIT |
| [rust-embed-impl](https://pyrossh.dev/repos/rust-embed) | 8.12.0 | MIT |
| [rust-embed-utils](https://pyrossh.dev/repos/rust-embed) | 8.12.0 | MIT |
| [rust-i18n](https://github.com/longbridge/rust-i18n) | 4.2.3 | MIT |
| [rust-i18n-macro](https://github.com/longbridge/rust-i18n) | 4.2.3 | MIT |
| [rust-i18n-support](https://github.com/longbridge/rust-i18n) | 4.2.3 | MIT |
| [rustc-demangle](https://github.com/rust-lang/rustc-demangle) | 0.1.28 | MIT/Apache-2.0 |
| [rustc-hash](https://github.com/rust-lang/rustc-hash) | 2.1.3 | Apache-2.0 OR MIT |
| [rustc_version](https://github.com/djc/rustc-version-rs) | 0.4.1 | MIT OR Apache-2.0 |
| [rustix](https://github.com/bytecodealliance/rustix) | 1.1.5 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| [rustversion](https://github.com/dtolnay/rustversion) | 1.0.23 | MIT OR Apache-2.0 |
| [rustybuzz](https://github.com/harfbuzz/rustybuzz) | 0.20.1 | MIT |
| [ryu](https://github.com/dtolnay/ryu) | 1.0.23 | Apache-2.0 OR BSL-1.0 |
| [same-file](https://github.com/BurntSushi/same-file) | 1.0.6 | Unlicense/MIT |
| [schemars](https://github.com/GREsau/schemars) | 1.2.2 | MIT |
| [schemars_derive](https://github.com/GREsau/schemars) | 1.2.2 | MIT |
| [scopeguard](https://github.com/bluss/scopeguard) | 1.2.0 | MIT OR Apache-2.0 |
| [seahash](https://gitlab.redox-os.org/redox-os/seahash) | 4.1.0 | MIT |
| [semver](https://github.com/dtolnay/semver) | 1.0.28 | MIT OR Apache-2.0 |
| [serde](https://github.com/serde-rs/serde) | 1.0.229 | MIT OR Apache-2.0 |
| [serde-saphyr](https://github.com/bourumir-wyngs/serde-saphyr) | 1.3.0 | MIT OR Apache-2.0 |
| [serde_core](https://github.com/serde-rs/serde) | 1.0.229 | MIT OR Apache-2.0 |
| [serde_derive](https://github.com/serde-rs/serde) | 1.0.229 | MIT OR Apache-2.0 |
| [serde_derive_internals](https://github.com/serde-rs/serde) | 0.30.0 | MIT OR Apache-2.0 |
| [serde_fmt](https://github.com/KodrAus/serde_fmt.git) | 1.1.0 | Apache-2.0 OR MIT |
| [serde_json](https://github.com/serde-rs/json) | 1.0.151 | MIT OR Apache-2.0 |
| [serde_repr](https://github.com/dtolnay/serde-repr) | 0.1.21 | MIT OR Apache-2.0 |
| [serde_spanned](https://github.com/toml-rs/toml) | 0.6.9 | MIT OR Apache-2.0 |
| [serde_spanned](https://github.com/toml-rs/toml) | 1.1.1 | MIT OR Apache-2.0 |
| [serde_urlencoded](https://github.com/nox/serde_urlencoded) | 0.7.1 | MIT/Apache-2.0 |
| [sha1_smol](https://github.com/mitsuhiko/sha1-smol) | 1.0.1 | BSD-3-Clause |
| [sha2](https://github.com/RustCrypto/hashes) | 0.11.0 | MIT OR Apache-2.0 |
| [sharded-slab](https://github.com/hawkw/sharded-slab) | 0.1.7 | MIT |
| [shellexpand](https://gitlab.com/ijackson/rust-shellexpand) | 3.1.2 | MIT/Apache-2.0 |
| [shlex](https://github.com/comex/rust-shlex) | 2.0.1 | MIT OR Apache-2.0 |
| [simd-adler32](https://github.com/mcountryman/simd-adler32) | 0.3.10 | MIT |
| [simd_helpers](https://github.com/lu-zero/simd_helpers) | 0.1.0 | MIT |
| [simdutf8](https://github.com/rusticstuff/simdutf8) | 0.1.5 | MIT OR Apache-2.0 |
| [similar](https://github.com/mitsuhiko/similar) | 2.7.0 | Apache-2.0 |
| [simplecss](https://github.com/linebender/simplecss) | 0.2.2 | Apache-2.0 OR MIT |
| [siphasher](https://github.com/jedisct1/rust-siphash) | 1.0.4 | MIT OR Apache-2.0 |
| [slab](https://github.com/tokio-rs/slab) | 0.4.12 | MIT |
| [slotmap](https://github.com/orlp/slotmap) | 1.1.1 | Zlib |
| [smallvec](https://github.com/servo/rust-smallvec) | 1.16.2 | MIT OR Apache-2.0 |
| [smol](https://github.com/smol-rs/smol) | 2.0.2 | Apache-2.0 OR MIT |
| [smol_str](https://github.com/rust-lang/rust-analyzer/tree/master/lib/smol_str) | 0.3.6 | MIT OR Apache-2.0 |
| [spin](https://github.com/mvdnes/spin-rs.git) | 0.10.1 | MIT |
| [spin](https://github.com/mvdnes/spin-rs.git) | 0.9.9 | MIT |
| [stable_deref_trait](https://github.com/storyyeller/stable_deref_trait) | 1.2.1 | MIT OR Apache-2.0 |
| [static_assertions](https://github.com/nvzqz/static-assertions-rs) | 1.1.0 | MIT OR Apache-2.0 |
| [str_indices](https://github.com/cessen/str_indices) | 0.4.4 | MIT OR Apache-2.0 |
| [streaming-iterator](https://github.com/sfackler/streaming-iterator) | 0.1.9 | MIT OR Apache-2.0 |
| [strict-num](https://github.com/RazrFalcon/strict-num) | 0.1.1 | MIT |
| [string_cache](https://github.com/servo/string-cache) | 0.8.9 | MIT OR Apache-2.0 |
| [string_cache_codegen](https://github.com/servo/string-cache) | 0.5.4 | MIT OR Apache-2.0 |
| [strum](https://github.com/Peternator7/strum) | 0.28.0 | MIT |
| [strum_macros](https://github.com/Peternator7/strum) | 0.28.0 | MIT |
| [sval](https://github.com/sval-rs/sval) | 2.22.0 | Apache-2.0 OR MIT |
| [sval_buffer](https://github.com/sval-rs/sval) | 2.22.0 | Apache-2.0 OR MIT |
| [sval_dynamic](https://github.com/sval-rs/sval) | 2.22.0 | Apache-2.0 OR MIT |
| [sval_fmt](https://github.com/sval-rs/sval) | 2.22.0 | Apache-2.0 OR MIT |
| [sval_json](https://github.com/sval-rs/sval) | 2.22.0 | Apache-2.0 OR MIT |
| [sval_nested](https://github.com/sval-rs/sval) | 2.22.0 | Apache-2.0 OR MIT |
| [sval_ref](https://github.com/sval-rs/sval) | 2.22.0 | Apache-2.0 OR MIT |
| [sval_serde](https://github.com/sval-rs/sval) | 2.22.0 | Apache-2.0 OR MIT |
| [svg_fmt](https://github.com/nical/rust_debug) | 0.4.5 | MIT/Apache-2.0 |
| [svgtypes](https://github.com/linebender/svgtypes) | 0.15.3 | Apache-2.0 OR MIT |
| [svgtypes](https://github.com/linebender/svgtypes) | 0.16.1 | Apache-2.0 OR MIT |
| [syn](https://github.com/dtolnay/syn) | 2.0.119 | MIT OR Apache-2.0 |
| [syn](https://github.com/dtolnay/syn) | 3.0.6 | MIT OR Apache-2.0 |
| [synstructure](https://github.com/mystor/synstructure) | 0.14.0 | MIT |
| [sysinfo](https://github.com/GuillaumeGomez/sysinfo) | 0.31.4 | MIT |
| [taffy](https://github.com/DioxusLabs/taffy) | 0.13.0 | MIT |
| [tempfile](https://github.com/Stebalien/tempfile) | 3.27.0 | MIT OR Apache-2.0 |
| [tendril](https://github.com/servo/tendril) | 0.4.3 | MIT/Apache-2.0 |
| [thiserror](https://github.com/dtolnay/thiserror) | 1.0.69 | MIT OR Apache-2.0 |
| [thiserror](https://github.com/dtolnay/thiserror) | 2.0.21 | MIT OR Apache-2.0 |
| [thiserror-impl](https://github.com/dtolnay/thiserror) | 1.0.69 | MIT OR Apache-2.0 |
| [thiserror-impl](https://github.com/dtolnay/thiserror) | 2.0.21 | MIT OR Apache-2.0 |
| [thread_local](https://github.com/Amanieu/thread_local-rs) | 1.1.10 | MIT OR Apache-2.0 |
| [tiff](https://github.com/image-rs/image-tiff) | 0.11.3 | MIT |
| [tiny-skia](https://github.com/RazrFalcon/tiny-skia) | 0.11.4 | BSD-3-Clause |
| [tiny-skia-path](https://github.com/RazrFalcon/tiny-skia/tree/master/path) | 0.11.4 | BSD-3-Clause |
| [tinystr](https://github.com/unicode-org/icu4x) | 0.8.4 | Unicode-3.0 |
| [tinyvec](https://github.com/Lokathor/tinyvec) | 1.13.3 | Zlib OR Apache-2.0 OR MIT |
| [toml](https://github.com/toml-rs/toml) | 0.8.23 | MIT OR Apache-2.0 |
| [toml](https://github.com/toml-rs/toml) | 0.9.12+spec-1.1.0 | MIT OR Apache-2.0 |
| [toml](https://github.com/toml-rs/toml) | 1.1.6+spec-1.1.0 | MIT OR Apache-2.0 |
| [toml_datetime](https://github.com/toml-rs/toml) | 0.6.11 | MIT OR Apache-2.0 |
| [toml_datetime](https://github.com/toml-rs/toml) | 0.7.5+spec-1.1.0 | MIT OR Apache-2.0 |
| [toml_datetime](https://github.com/toml-rs/toml) | 1.1.1+spec-1.1.0 | MIT OR Apache-2.0 |
| [toml_edit](https://github.com/toml-rs/toml) | 0.22.27 | MIT OR Apache-2.0 |
| [toml_edit](https://github.com/toml-rs/toml) | 0.25.15+spec-1.1.0 | MIT OR Apache-2.0 |
| [toml_parser](https://github.com/toml-rs/toml) | 1.1.3+spec-1.1.0 | MIT OR Apache-2.0 |
| [toml_write](https://github.com/toml-rs/toml) | 0.1.2 | MIT OR Apache-2.0 |
| [toml_writer](https://github.com/toml-rs/toml) | 1.1.2+spec-1.1.0 | MIT OR Apache-2.0 |
| [tracing](https://github.com/tokio-rs/tracing) | 0.1.44 | MIT |
| [tracing-attributes](https://github.com/tokio-rs/tracing) | 0.1.31 | MIT |
| [tracing-core](https://github.com/tokio-rs/tracing) | 0.1.36 | MIT |
| [tracing-log](https://github.com/tokio-rs/tracing) | 0.2.0 | MIT |
| [tracing-subscriber](https://github.com/tokio-rs/tracing) | 0.3.23 | MIT |
| [tree-sitter](https://github.com/tree-sitter/tree-sitter) | 0.26.13 | MIT |
| [tree-sitter-bash](https://github.com/tree-sitter/tree-sitter-bash) | 0.23.3 | MIT |
| [tree-sitter-css](https://github.com/tree-sitter/tree-sitter-css) | 0.23.2 | MIT |
| [tree-sitter-go](https://github.com/tree-sitter/tree-sitter-go) | 0.23.4 | MIT |
| [tree-sitter-html](https://github.com/tree-sitter/tree-sitter-html) | 0.23.2 | MIT |
| [tree-sitter-javascript](https://github.com/tree-sitter/tree-sitter-javascript) | 0.23.1 | MIT |
| [tree-sitter-json](https://github.com/tree-sitter/tree-sitter-json) | 0.24.8 | MIT |
| [tree-sitter-language](https://github.com/tree-sitter/tree-sitter) | 0.1.8 | MIT |
| [tree-sitter-md](https://github.com/tree-sitter-grammars/tree-sitter-markdown) | 0.5.3 | MIT |
| [tree-sitter-python](https://github.com/tree-sitter/tree-sitter-python) | 0.23.6 | MIT |
| [tree-sitter-rust](https://github.com/tree-sitter/tree-sitter-rust) | 0.24.2 | MIT |
| [tree-sitter-toml-ng](https://github.com/tree-sitter-grammars/tree-sitter-toml) | 0.7.0 | MIT |
| [tree-sitter-typescript](https://github.com/tree-sitter/tree-sitter-typescript) | 0.23.2 | MIT |
| [triomphe](https://github.com/Manishearth/triomphe) | 0.1.16 | MIT OR Apache-2.0 |
| [ttf-parser](https://github.com/harfbuzz/ttf-parser) | 0.25.1 | MIT OR Apache-2.0 |
| [typeid](https://github.com/dtolnay/typeid) | 1.0.3 | MIT OR Apache-2.0 |
| [typenum](https://github.com/paholg/typenum) | 1.20.1 | MIT OR Apache-2.0 |
| [unicase](https://github.com/seanmonstar/unicase) | 2.9.0 | MIT OR Apache-2.0 |
| [unicode-bidi](https://github.com/servo/unicode-bidi) | 0.3.18 | MIT OR Apache-2.0 |
| [unicode-bidi-mirroring](https://github.com/RazrFalcon/unicode-bidi-mirroring) | 0.4.0 | MIT/Apache-2.0 |
| [unicode-ccc](https://github.com/RazrFalcon/unicode-ccc) | 0.4.0 | MIT/Apache-2.0 |
| [unicode-id](https://github.com/Boshen/unicode-id) | 0.3.7 | MIT OR Apache-2.0 |
| [unicode-ident](https://github.com/dtolnay/unicode-ident) | 1.0.26 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| [unicode-properties](https://github.com/unicode-rs/unicode-properties) | 0.1.4 | MIT/Apache-2.0 |
| [unicode-script](https://github.com/unicode-rs/unicode-script) | 0.5.8 | MIT OR Apache-2.0 |
| [unicode-segmentation](https://github.com/unicode-rs/unicode-segmentation) | 1.13.3 | MIT OR Apache-2.0 |
| [unicode-vo](https://github.com/RazrFalcon/unicode-vo) | 0.1.0 | MIT/Apache-2.0 |
| [unicode-width](https://github.com/unicode-rs/unicode-width) | 0.2.2 | MIT OR Apache-2.0 |
| [unicode-xid](https://github.com/unicode-rs/unicode-xid) | 0.2.6 | MIT OR Apache-2.0 |
| [url](https://github.com/servo/rust-url) | 2.5.8 | MIT OR Apache-2.0 |
| [usvg](https://github.com/linebender/resvg) | 0.45.1 | Apache-2.0 OR MIT |
| [usvg](https://github.com/linebender/resvg) | 0.46.0 | Apache-2.0 OR MIT |
| [utf-8](https://github.com/SimonSapin/rust-utf8) | 0.7.6 | MIT OR Apache-2.0 |
| [utf8_iter](https://github.com/hsivonen/utf8_iter) | 1.0.4 | Apache-2.0 OR MIT |
| [utf8parse](https://github.com/alacritty/vte) | 0.2.2 | Apache-2.0 OR MIT |
| [uuid](https://github.com/uuid-rs/uuid) | 1.26.1 | Apache-2.0 OR MIT |
| [v_frame](https://github.com/rust-av/v_frame) | 0.3.9 | BSD-2-Clause |
| [value-bag](https://github.com/sval-rs/value-bag) | 1.14.1 | Apache-2.0 OR MIT |
| [value-bag-serde1](https://crates.io/crates/value-bag-serde1) | 1.14.1 | Apache-2.0 OR MIT |
| [value-bag-sval2](https://crates.io/crates/value-bag-sval2) | 1.14.1 | Apache-2.0 OR MIT |
| [version_check](https://github.com/SergioBenitez/version_check) | 0.9.5 | MIT/Apache-2.0 |
| [vswhom](https://github.com/nabijaczleweli/vswhom.rs) | 0.1.0 | MIT |
| [vswhom-sys](https://github.com/nabijaczleweli/vswhom-sys.rs) | 0.1.3 | MIT |
| [vte](https://github.com/alacritty/vte) | 0.15.0 | Apache-2.0 OR MIT |
| [waker-fn](https://github.com/smol-rs/waker-fn) | 1.2.0 | Apache-2.0 OR MIT |
| [walkdir](https://github.com/BurntSushi/walkdir) | 2.5.0 | Unlicense/MIT |
| [wasm-bindgen](https://github.com/wasm-bindgen/wasm-bindgen) | 0.2.129 | MIT OR Apache-2.0 |
| [wasm-bindgen-macro](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/macro) | 0.2.129 | MIT OR Apache-2.0 |
| [wasm-bindgen-macro-support](https://github.com/wasm-bindgen/wasm-bindgen/tree/main/crates/macro-support) | 0.2.129 | MIT OR Apache-2.0 |
| [wasm-bindgen-shared](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/shared) | 0.2.129 | MIT OR Apache-2.0 |
| [web-time](https://github.com/daxpedda/web-time) | 1.1.0 | MIT OR Apache-2.0 |
| [weezl](https://github.com/image-rs/weezl) | 0.1.12 | MIT OR Apache-2.0 |
| [which](https://github.com/harryfei/which-rs.git) | 8.0.6 | MIT |
| [winapi](https://github.com/retep998/winapi-rs) | 0.3.9 | MIT/Apache-2.0 |
| [winapi-util](https://github.com/BurntSushi/winapi-util) | 0.1.11 | Unlicense OR MIT |
| [windows](https://github.com/microsoft/windows-rs) | 0.57.0 | MIT OR Apache-2.0 |
| [windows](https://github.com/microsoft/windows-rs) | 0.58.0 | MIT OR Apache-2.0 |
| [windows](https://github.com/microsoft/windows-rs) | 0.61.3 | MIT OR Apache-2.0 |
| [windows](https://github.com/microsoft/windows-rs) | 0.62.2 | MIT OR Apache-2.0 |
| [windows-capture](https://github.com/NiiightmareXD/windows-capture) | 1.5.0 | MIT |
| [windows-collections](https://github.com/microsoft/windows-rs) | 0.2.0 | MIT OR Apache-2.0 |
| [windows-collections](https://github.com/microsoft/windows-rs) | 0.3.2 | MIT OR Apache-2.0 |
| [windows-core](https://github.com/microsoft/windows-rs) | 0.57.0 | MIT OR Apache-2.0 |
| [windows-core](https://github.com/microsoft/windows-rs) | 0.58.0 | MIT OR Apache-2.0 |
| [windows-core](https://github.com/microsoft/windows-rs) | 0.61.2 | MIT OR Apache-2.0 |
| [windows-core](https://github.com/microsoft/windows-rs) | 0.62.2 | MIT OR Apache-2.0 |
| [windows-future](https://github.com/microsoft/windows-rs) | 0.2.1 | MIT OR Apache-2.0 |
| [windows-future](https://github.com/microsoft/windows-rs) | 0.3.2 | MIT OR Apache-2.0 |
| [windows-implement](https://github.com/microsoft/windows-rs) | 0.57.0 | MIT OR Apache-2.0 |
| [windows-implement](https://github.com/microsoft/windows-rs) | 0.58.0 | MIT OR Apache-2.0 |
| [windows-implement](https://github.com/microsoft/windows-rs) | 0.60.2 | MIT OR Apache-2.0 |
| [windows-interface](https://github.com/microsoft/windows-rs) | 0.57.0 | MIT OR Apache-2.0 |
| [windows-interface](https://github.com/microsoft/windows-rs) | 0.58.0 | MIT OR Apache-2.0 |
| [windows-interface](https://github.com/microsoft/windows-rs) | 0.59.3 | MIT OR Apache-2.0 |
| [windows-link](https://github.com/microsoft/windows-rs) | 0.1.3 | MIT OR Apache-2.0 |
| [windows-link](https://github.com/microsoft/windows-rs) | 0.2.1 | MIT OR Apache-2.0 |
| [windows-numerics](https://github.com/microsoft/windows-rs) | 0.2.0 | MIT OR Apache-2.0 |
| [windows-numerics](https://github.com/microsoft/windows-rs) | 0.3.1 | MIT OR Apache-2.0 |
| [windows-registry](https://github.com/microsoft/windows-rs) | 0.6.1 | MIT OR Apache-2.0 |
| [windows-result](https://github.com/microsoft/windows-rs) | 0.1.2 | MIT OR Apache-2.0 |
| [windows-result](https://github.com/microsoft/windows-rs) | 0.2.0 | MIT OR Apache-2.0 |
| [windows-result](https://github.com/microsoft/windows-rs) | 0.3.4 | MIT OR Apache-2.0 |
| [windows-result](https://github.com/microsoft/windows-rs) | 0.4.1 | MIT OR Apache-2.0 |
| [windows-strings](https://github.com/microsoft/windows-rs) | 0.1.0 | MIT OR Apache-2.0 |
| [windows-strings](https://github.com/microsoft/windows-rs) | 0.4.2 | MIT OR Apache-2.0 |
| [windows-strings](https://github.com/microsoft/windows-rs) | 0.5.1 | MIT OR Apache-2.0 |
| [windows-sys](https://github.com/microsoft/windows-rs) | 0.52.0 | MIT OR Apache-2.0 |
| [windows-sys](https://github.com/microsoft/windows-rs) | 0.59.0 | MIT OR Apache-2.0 |
| [windows-sys](https://github.com/microsoft/windows-rs) | 0.60.2 | MIT OR Apache-2.0 |
| [windows-sys](https://github.com/microsoft/windows-rs) | 0.61.2 | MIT OR Apache-2.0 |
| [windows-targets](https://github.com/microsoft/windows-rs) | 0.52.6 | MIT OR Apache-2.0 |
| [windows-targets](https://github.com/microsoft/windows-rs) | 0.53.5 | MIT OR Apache-2.0 |
| [windows-threading](https://github.com/microsoft/windows-rs) | 0.1.0 | MIT OR Apache-2.0 |
| [windows-threading](https://github.com/microsoft/windows-rs) | 0.2.1 | MIT OR Apache-2.0 |
| [windows_x86_64_msvc](https://github.com/microsoft/windows-rs) | 0.52.6 | MIT OR Apache-2.0 |
| [windows_x86_64_msvc](https://github.com/microsoft/windows-rs) | 0.53.1 | MIT OR Apache-2.0 |
| [winnow](https://github.com/winnow-rs/winnow) | 0.7.15 | MIT |
| [winnow](https://github.com/winnow-rs/winnow) | 1.0.4 | MIT |
| [winreg](https://github.com/gentoo90/winreg-rs) | 0.55.0 | MIT |
| [writeable](https://github.com/unicode-org/icu4x) | 0.6.4 | Unicode-3.0 |
| [xml5ever](https://github.com/servo/html5ever) | 0.18.1 | MIT OR Apache-2.0 |
| [xmlwriter](https://github.com/RazrFalcon/xmlwriter) | 0.1.0 | MIT |
| [y4m](https://github.com/image-rs/y4m.git) | 0.8.0 | MIT |
| [yoke](https://github.com/unicode-org/icu4x) | 0.8.3 | Unicode-3.0 |
| [yoke-derive](https://github.com/unicode-org/icu4x) | 0.8.3 | Unicode-3.0 |
| [zed-scap](https://github.com/helmerapp/scap) | 0.0.8-zed | MIT |
| [zerocopy](https://github.com/google/zerocopy) | 0.8.59 | BSD-2-Clause OR Apache-2.0 OR MIT |
| [zerocopy-derive](https://github.com/google/zerocopy) | 0.8.59 | BSD-2-Clause OR Apache-2.0 OR MIT |
| [zerofrom](https://github.com/unicode-org/icu4x) | 0.1.8 | Unicode-3.0 |
| [zerofrom-derive](https://github.com/unicode-org/icu4x) | 0.1.8 | Unicode-3.0 |
| [zerotrie](https://github.com/unicode-org/icu4x) | 0.2.5 | Unicode-3.0 |
| [zerovec](https://github.com/unicode-org/icu4x) | 0.11.8 | Unicode-3.0 |
| [zerovec-derive](https://github.com/unicode-org/icu4x) | 0.11.6 | Unicode-3.0 |
| [zlib-rs](https://github.com/trifectatechfoundation/zlib-rs) | 0.6.8 | Zlib |
| [zmij](https://github.com/dtolnay/zmij) | 1.0.23 | MIT |
| [zune-core](https://crates.io/crates/zune-core) | 0.4.12 | MIT OR Apache-2.0 OR Zlib |
| [zune-core](https://github.com/etemesi254/zune-image) | 0.5.3 | MIT OR Apache-2.0 OR Zlib |
| [zune-inflate](https://crates.io/crates/zune-inflate) | 0.2.54 | MIT OR Apache-2.0 OR Zlib |
| [zune-jpeg](https://github.com/etemesi254/zune-image/tree/dev/crates/zune-jpeg) | 0.4.21 | MIT OR Apache-2.0 OR Zlib |
| [zune-jpeg](https://github.com/etemesi254/zune-image/tree/dev/crates/zune-jpeg) | 0.5.15 | MIT OR Apache-2.0 OR Zlib |
