#!/usr/bin/env bash
# Phase 0 measurement for xdr-snip v0.6.0 (issue #7).
#
# Produces the number the gate actually needs: a TRIMMED, ENCODER-ONLY libjxl
# statically linked into a minimal Windows PE that encodes one frame.
#
# Compare against:
#   xdr-snip v0.5.0 exe   = 3,068,928 B   (the budget)
#   upstream cjxl.exe     = 5,250,560 B   (FAT upper bound, all SIMD + CLI + decoder)
#
# RUN 1 FAILURES FIXED HERE:
#  1. Link died on `undefined reference to __imp_Jxl*`. The __imp_ prefix is the
#     MinGW marker for __declspec(dllimport): jxl_export.h assumes a DLL unless
#     JXL_STATIC_DEFINE is defined. Static linking REQUIRES -DJXL_STATIC_DEFINE.
#  2. ninja died at [188/196] enc_fast_lossless.cc.obj but only printed the
#     generic "subcommand failed" summary because output was piped to `tail`.
#     Now the build log is tee'd to a file and the real diagnostic is extracted.
#  3. MEASURE_EXIT_CODE was reported as 0 despite failure ($? of the wrong
#     pipeline stage). Now every stage's status is captured explicitly.
set -uo pipefail

BUDGET=3068928
CJXL_FAT=5250560
TC=/work/mingw-toolchain.cmake
BUILD_LOG=/work/build.log

cat > "$TC" <<'EOF'
set(CMAKE_SYSTEM_NAME Windows)
set(CMAKE_SYSTEM_PROCESSOR x86_64)
set(CMAKE_C_COMPILER   x86_64-w64-mingw32-gcc)
set(CMAKE_CXX_COMPILER x86_64-w64-mingw32-g++)
set(CMAKE_RC_COMPILER  x86_64-w64-mingw32-windres)
set(CMAKE_FIND_ROOT_PATH /usr/x86_64-w64-mingw32)
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
EOF

echo "############ CONFIGURE (trim profile per issue #7 §0) ############"
cmake -S /work/libjxl -B /work/build -G Ninja \
  -DCMAKE_TOOLCHAIN_FILE="$TC" \
  -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_SHARED_LIBS=OFF \
  -DBUILD_TESTING=OFF \
  -DJPEGXL_STATIC=ON \
  -DJPEGXL_ENABLE_TOOLS=OFF \
  -DJPEGXL_ENABLE_BENCHMARK=OFF \
  -DJPEGXL_ENABLE_EXAMPLES=OFF \
  -DJPEGXL_ENABLE_PLUGINS=OFF \
  -DJPEGXL_ENABLE_MANPAGES=OFF \
  -DJPEGXL_ENABLE_DOXYGEN=OFF \
  -DJPEGXL_ENABLE_JNI=OFF \
  -DJPEGXL_ENABLE_SJPEG=OFF \
  -DJPEGXL_ENABLE_OPENEXR=OFF \
  -DJPEGXL_ENABLE_TRANSCODE_JPEG=OFF \
  -DJPEGXL_ENABLE_BOXES=OFF \
  -DJPEGXL_ENABLE_SKCMS=ON \
  -DJPEGXL_ENABLE_FUZZERS=OFF \
  -DJPEGXL_ENABLE_DEVTOOLS=OFF \
  -DJPEGXL_ENABLE_VIEWERS=OFF \
  -DJPEGXL_ENABLE_HWY_AVX3=OFF \
  -DJPEGXL_ENABLE_HWY_AVX3_DL=OFF \
  -DJPEGXL_ENABLE_HWY_AVX3_SPR=OFF \
  -DJPEGXL_ENABLE_HWY_AVX3_ZEN4=OFF \
  -DJPEGXL_ENABLE_HWY_SSSE3=OFF \
  -DJPEGXL_ENABLE_HWY_SSE2=OFF \
  > /work/configure.log 2>&1
CONFIGURE_RC=$?
tail -20 /work/configure.log
echo "CONFIGURE_RC=$CONFIGURE_RC"
[ $CONFIGURE_RC -ne 0 ] && { echo "CONFIGURE FAILED - aborting, no number to report"; exit 1; }

echo "############ BUILD (full log -> $BUILD_LOG) ############"
# -k 0 keeps going after a failure so we learn EVERY broken target, not just the first.
cmake --build /work/build --parallel "$(nproc)" -- -k 0 > "$BUILD_LOG" 2>&1
BUILD_RC=$?
echo "BUILD_RC=$BUILD_RC"
tail -12 "$BUILD_LOG"

if [ $BUILD_RC -ne 0 ]; then
  echo "---------- ACTUAL COMPILER DIAGNOSTICS (not ninja's generic summary) ----------"
  grep -nE 'error:|Error [0-9]|FAILED:' "$BUILD_LOG" | head -40
  echo "---------- context around first FAILED ----------"
  awk '/^FAILED:/{found=NR} found && NR>=found && NR<=found+25' "$BUILD_LOG" | head -30
fi

echo "############ STATIC LIB SIZES (mingw .a) ############"
find /work/build -name '*.a' -printf '%10s  %p\n' 2>/dev/null | sort -rn | head -20

echo "############ MINIMAL ENCODE-ONE-FRAME PE ############"
cat > /work/mini.c <<'EOF'
#include <jxl/encode.h>
#include <stdlib.h>
#include <string.h>
/* Minimal realistic use: encode one 64x64 RGB frame. Mirrors what xdr-snip
   would link — encoder API only, no decoder, no CLI, no extras IO. */
int main(void) {
    JxlEncoder *e = JxlEncoderCreate(NULL);
    if (!e) return 1;
    JxlBasicInfo bi; JxlEncoderInitBasicInfo(&bi);
    bi.xsize = 64; bi.ysize = 64;
    bi.bits_per_sample = 8; bi.num_color_channels = 3;
    bi.uses_original_profile = JXL_FALSE;
    JxlEncoderSetBasicInfo(e, &bi);
    JxlColorEncoding ce; JxlColorEncodingSetToSRGB(&ce, JXL_FALSE);
    JxlEncoderSetColorEncoding(e, &ce);
    JxlEncoderFrameSettings *fs = JxlEncoderFrameSettingsCreate(e, NULL);
    JxlPixelFormat pf = {3, JXL_TYPE_UINT8, JXL_NATIVE_ENDIAN, 0};
    static unsigned char px[64*64*3];
    memset(px, 128, sizeof px);
    JxlEncoderAddImageFrame(fs, &pf, px, sizeof px);
    JxlEncoderCloseInput(e);
    static unsigned char out[1<<20];
    unsigned char *next = out; size_t avail = sizeof out;
    while (JxlEncoderProcessOutput(e, &next, &avail) == JXL_ENC_NEED_MORE_OUTPUT) {}
    JxlEncoderDestroy(e);
    return 0;
}
EOF

# Link order matters for static archives: enc before dec before common deps.
LIBS=""
for want in libjxl_enc.a libjxl.a libjxl_dec.a libjxl_cms.a libhwy.a \
            libbrotlienc.a libbrotlidec.a libbrotlicommon.a libjxl_threads.a; do
    f=$(find /work/build -name "$want" | head -1)
    [ -n "$f" ] && LIBS="$LIBS $f"
done
echo "LINKING AGAINST:$LIBS"

INC="-I/work/libjxl/lib/include -I/work/build/lib/include"
# JXL_STATIC_DEFINE is REQUIRED: without it jxl_export.h emits __declspec(dllimport)
# and every symbol resolves as __imp_Jxl* against a DLL that does not exist.
x86_64-w64-mingw32-gcc -Os -DJXL_STATIC_DEFINE -o /work/mini.exe /work/mini.c \
    $INC $LIBS -lstdc++ -lpthread -static -static-libgcc -static-libstdc++ \
    > /work/link.log 2>&1
LINK_RC=$?
echo "LINK_RC=$LINK_RC"
[ $LINK_RC -ne 0 ] && tail -25 /work/link.log

if [ -f /work/mini.exe ] && [ $LINK_RC -eq 0 ]; then
  x86_64-w64-mingw32-strip /work/mini.exe
  SZ=$(stat -c%s /work/mini.exe)
  echo "=================== RESULT ==================="
  echo "mini.exe (trimmed encoder-only, stripped) = $SZ bytes"
  echo "xdr-snip v0.5.0 budget                    = $BUDGET bytes"
  echo "upstream cjxl.exe (FAT upper bound)       = $CJXL_FAT bytes"
  python3 - "$SZ" <<'PY'
import sys
sz=int(sys.argv[1]); budget=3068928; fat=5250560
print(f"trimmed vs FAT cjxl : {sz/fat:.2f}x  (saved {fat-sz:,} B)")
print(f"trimmed vs budget   : {sz/budget:.2f}x")
head = budget - sz
print(f"HEADROOM left for ALL of xdr-snip (Win32 UI, D3D11/WinRT capture, WIC, libwebp, QOI, static CRT): {head:,} B")
print("VERDICT-INPUT: " + ("PLAUSIBLE - headroom remains" if head > 0 else "OVER BUDGET on libjxl alone"))
PY
  FINAL_RC=0
else
  echo "LINK FAILED or mini.exe missing - report errors verbatim, do NOT fabricate a number"
  FINAL_RC=1
fi

echo "MEASURE_EXIT_CODE=$FINAL_RC"
exit $FINAL_RC
