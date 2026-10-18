# Synthetic movie regression fixture

`counter.omv` contains three seconds of a generated changing color pattern at
64x48 / 30 fps, encoded as Theora. It contains no game data. Its minimal
88-byte OMV header has type 2 (YUV), a 33,333 us frame time and no seek index;
this also exercises incremental playback without a table-derived duration.

Generate the Ogg payload with:

```sh
ffmpeg -f lavfi \
  -i 'nullsrc=size=64x48:rate=30:duration=3,geq=lum=mod(X+N*7\,256):cb=mod(Y+N*5\,256):cr=mod(X+Y+N*3\,256)' \
  -c:v libtheora -q:v 10 -g 1 -an counter.ogg
```

The regression checks actual decoded pixels, EOF/duration, loop restart,
per-poll work limits and retained frame storage. No external game installation
or ffmpeg executable is required to run the tests.

The generated pattern changes every frame and uses all intra frames so the
Ogg encoder does not coalesce still images into granule-position gaps. The
minimal header has no packet timestamp table to describe such gaps. Native
debug builds of the Theora decoder need a larger test-thread stack; run this
focused regression with `RUST_MIN_STACK=16777216 cargo test -p siglus_scene_vm
--lib movie::browser_omv::tests`.

`duplicated.omv` repacks those packets into individual Ogg pages with valid
CRC-32 checksums and inserts an empty Theora duplicate packet after each data
packet. It has 180 presentation packets but only 90 new pictures. This checks
that a future decoded picture stays buffered until its presentation time and
that duplicate packets still count toward duration.

`counter.wmv` is the three-second 64x48 / 30 fps `testsrc2` pattern encoded
with FFmpeg `-c:v wmv2 -an`. It is used by the standalone browser memory probe
to reproduce the old Web WMV full-frame cache without loading a large game
movie. It now also checks bounded playback, EOF and looping.


`counter.mpg` contains the same 64x48 / 30 fps / 3 s pattern, MPEG-2 with two
B pictures between reference pictures, plus a 440 Hz mono MPEG Layer II track
at 32 kHz. Generate it with:

```sh
ffmpeg -f lavfi -i 'testsrc2=size=64x48:rate=30:duration=3' \
  -f lavfi -i 'sine=frequency=440:sample_rate=32000:duration=3' \
  -c:v mpeg2video -bf 2 -g 15 -c:a mp2 -b:a 64k -f mpeg counter.mpg
```

`movie::browser_video::tests` checks all 90 pictures (including delayed EOF
pictures), queue bounds, embedded audio samples, large clock jumps and loop
restart. The browser probe additionally checks preview size, the disabled
full-film API, idle eviction/reopening and zero retained video after stop.


`no-duration.wmv` is a copy of the generated `counter.wmv` with the ASF File
Properties object's play/send duration fields (64/72 bytes from its GUID)
cleared. The media payload is unchanged. It verifies EOF duration discovery
and looping for streams without header duration metadata.
