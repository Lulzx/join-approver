# Join Approver demo video

The 26-second demo in the main README, made with
[fframes](https://github.com/dmtrKovalenko/fframes) (Skia on Metal):
1920x1080 at 30 fps, with an original beat.

| file | what |
| --- | --- |
| `src/lib.rs` | the video: scenes, animation, audio map |
| `src/shaders/` | background shaders ported from [Shader Effects](https://github.com/shader-effects-inc/shaders) (MIT) |
| `scripts/beat.py` | synthesizes the beat into `media/beat.wav` (no samples) |
| `scripts/screens.ts` | screenshots of the real UI for the video, `media/app-*.png` |
| `scripts/docs-shots.ts` | the README screenshots, `../docs/*.png` |
| `media/` | fonts, images and audio compiled into the binary |

The screenshots come from the app's built UI (`bun run build` in the repo
root) with a mocked Tauri bridge, so every channel and person shown is made
up.

## Build

Needs Rust, ffmpeg (`brew install pkg-config ffmpeg x264 x265 opus nasm ninja`),
Python 3 with numpy, and Bun with Chrome for the screenshots.

```sh
python3 scripts/beat.py                 # media/beat.wav, not committed
bun install && bun scripts/screens.ts   # only if the UI changed
cargo run --release -- render -o out.mp4
cargo run --release -- preview          # watch it with sound
```

For the README copy, render without film grain (noise doesn't compress) and
encode small:

```sh
GRAIN=0 cargo run --release -- render -o web.mp4
ffmpeg -i web.mp4 -c:v libx264 -preset veryslow -crf 28 -pix_fmt yuv420p \
  -movflags +faststart -c:a aac -b:a 96k ../docs/demo.mp4
ffmpeg -i web.mp4 -vf "fps=10,scale=720:-2:flags=lanczos" /tmp/f%04d.png
img2webp -loop 0 -lossy -q 45 -m 4 -kmin 3 -kmax 5 -d 100 /tmp/f*.png -o ../docs/demo.webp
```

Other commands: `timeline`, `inspect`, `strip <scene> -n 12`,
`frame Intro@2s`, `audio analyze`.

## Fonts

Inter and DM Sans, under the SIL Open Font License 1.1 (the license text is
embedded in each font file). The Inter files have their family names changed
to one per weight (`Inter Black` and so on) so the renderer picks the right
weight.
