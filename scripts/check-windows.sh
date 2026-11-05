#!/usr/bin/env bash
# Checa (sem linkar) todo o código Windows a partir do Linux, inclusive o que
# depende do FFmpeg: bindgen roda com o alvo mingw usando os headers do FFmpeg
# do sistema (apt: libav*-dev). O build de verdade continua sendo o do CI (MSVC).
set -euo pipefail
FF=/tmp/app-ffwin
if [ ! -d "$FF/include/libavcodec" ]; then
  mkdir -p "$FF/include" "$FF/lib"
  for d in /usr/include/x86_64-linux-gnu/libav* /usr/include/x86_64-linux-gnu/libsw*; do
    cp -r "$d" "$FF/include/"
  done
fi
export FFMPEG_DIR="$FF"
# Intrínsecos AMX do mingw quebram o layout gerado pelo bindgen; o FFmpeg não precisa deles.
export BINDGEN_EXTRA_CLANG_ARGS="-D__AMXINTRIN_H -D__IMMINTRIN_H -D__X86INTRIN_H -D_INC_INTRIN"
cargo "${CARGO_CMD:-check}" --target x86_64-pc-windows-gnu -p app-platform-win -p app-media -p app-ipc -p app-recorder -p app-ui "$@"
