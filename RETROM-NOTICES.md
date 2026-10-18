# Retrom Siglus browser distribution

Engine: xmoezzz/siglus_rs, baseline 5f358ef1360b0439e92aed2b749e8f25d8721954.
The original engine and Retrom modifications are MPL-2.0 except where a file or
dependency carries a different license. LICENSE-MPL-2.0 accompanies this package.
This distribution contains no games, game executables, DLLs or game keys.

Exact corresponding source, dependency sources and rebuild/relink instructions
are published as siglus-source.tar.gz alongside each immutable core release at
https://github.com/retrom-project/siglus_rs/releases . The accompanying
rpg-runtime-release.json identifies the source commit and all binary/source
SHA-256 digests. RETROM.md describes rebuilding a modified decoder and replacing
the browser module. Modification and reverse engineering for debugging those
modifications are permitted under the applicable licenses; no additional
restriction is imposed by Retrom. The source archive includes the locked Cargo
registry sources, patched local dependencies and licensed default font.

## Codec attribution

crates/na_wmv_player contains Rust translations of FFmpeg WMA and VC-1 code and
tables, including wmaprodec.c, wmaprodata.h, wmadec.c, wmadata.h, vc1.c,
vc1data.c, vc1data.h, vc1_vlc_data.h, vc1dsp.c, vc1_loopfilter.c, vc1_mc.c and
simple_idct.c. These portions retain LGPL-2.1-or-later; the full LGPL-2.1 and
GPL-2.0 texts and the referenced FFmpeg copyright notices follow below.
The Rust port's exact corresponding source is the release's source commit;
FFmpeg n8.0 is an attribution reference, not a claimed original port revision.
Retrom changes include bounded browser movie decoding and resource cleanup.

crates/theora-rs is the upstream Rust rewrite of Xiph.Org libtheora 1.2.0.
Xiph's BSD copyright/license and On2 patent statement follow below. The Rust
sources retain the repository's MPL-2.0 terms and applicable upstream notices.

## Font and local dependencies

The embedded default font is Noto Sans SC, SIL Open Font License 1.1, from
google/fonts a85815a42757630ce188fdad368c2dfc444d4773, SHA-256
a3041811a78c361b1de50f953c805e0244951c21c5bd412f7232ef0d899af0da.
FONT-OFL.txt accompanies the binary. The upstream MS PGothic font is removed
from the maintenance branch and source release. Epaint's separately embedded
fonts retain their OFL/UFL/MIT notices reproduced below.

wgpu-hal 0.21.1 (MIT OR Apache-2.0) is modified to respect browser attachment
limits. CPAL 0.15.3 (Apache-2.0) is modified to stop queued callbacks after stream
destruction. Eluna is vendored under MPL-2.0. The complete normal/build dependency
inventory below is derived from Cargo.lock for wasm32-unknown-unknown.
