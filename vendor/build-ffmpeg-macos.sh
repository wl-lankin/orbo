#!/bin/zsh
# Builds an LGPL ffmpeg/ffprobe for macOS (arm64) with VideoToolbox, AudioToolbox and LAME (MP3).
# No GPL parts (no x264/x265), no Homebrew libraries: only system frameworks and static LAME.
set -euo pipefail
export MACOSX_DEPLOYMENT_TARGET=14.0
cd "${0:A:h}"
FFMPEG=7.1
LAME=3.100
LAME_SHA=ddfe36cab873794038ae2c1210557ad34857a4b6bdc515785d1da9e175b1da1e
ROOT=$PWD
SRC=$ROOT/src
PREFIX=$ROOT/prefix
OUT=$ROOT/../core/crates/orbo-app/bin
mkdir -p "$SRC" "$PREFIX" "$OUT"

cd "$SRC"
[[ -f lame-$LAME.tar.gz ]] || curl -fL -o lame-$LAME.tar.gz "https://downloads.sourceforge.net/project/lame/lame/$LAME/lame-$LAME.tar.gz"
echo "$LAME_SHA  lame-$LAME.tar.gz" | shasum -a 256 -c -
[[ -f ffmpeg-$FFMPEG.tar.xz ]] || curl -fL -o ffmpeg-$FFMPEG.tar.xz "https://ffmpeg.org/releases/ffmpeg-$FFMPEG.tar.xz"
shasum -a 256 ffmpeg-$FFMPEG.tar.xz

rm -rf lame-$LAME ffmpeg-$FFMPEG
tar xzf lame-$LAME.tar.gz
tar xJf ffmpeg-$FFMPEG.tar.xz

cd lame-$LAME
# lame 3.100 exports a symbol that no longer exists; the linker rejects it on current macOS.
sed -i '' '/lame_init_old/d' include/libmp3lame.sym
./configure --prefix="$PREFIX" --disable-shared --enable-static --disable-frontend --disable-dependency-tracking >/dev/null
make -j"$(sysctl -n hw.ncpu)" >/dev/null
make install >/dev/null
cd ..

cd ffmpeg-$FFMPEG
PKG_CONFIG_PATH= ./configure \
  --prefix="$PREFIX" \
  --disable-autodetect \
  --enable-videotoolbox --enable-audiotoolbox --enable-zlib \
  --enable-libmp3lame \
  --disable-gpl --disable-nonfree \
  --disable-doc --disable-ffplay --disable-debug \
  --enable-static --disable-shared \
  --extra-cflags="-I$PREFIX/include -mmacosx-version-min=14.0" \
  --extra-ldflags="-L$PREFIX/lib -mmacosx-version-min=14.0" >/dev/null
make -j"$(sysctl -n hw.ncpu)" >/dev/null
cp ffmpeg ffprobe "$OUT/"
cd "$OUT"
for b in ffmpeg ffprobe; do
  strip -x "$b" || true
  cp "$b" "$b-aarch64-apple-darwin"
done
echo "--- Bibliotheken:"
otool -L ffmpeg | tail -n +2
echo "--- Lizenz:"
./ffmpeg -hide_banner -L 2>&1 | head -3
echo "FERTIG: $OUT"
