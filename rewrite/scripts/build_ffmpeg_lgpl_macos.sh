#!/bin/bash
# Build on an Apple Silicon Mac with Xcode Command Line Tools, Python 3 and xz.
# Usage: bash rewrite/scripts/build_ffmpeg_lgpl_macos.sh /absolute/new/output-dir
# No upload, signing service, or app pin is changed by this script.
set -euo pipefail
export LC_ALL=C TZ=UTC
umask 022
[[ $(uname -s) == Darwin && $(uname -m) == arm64 ]] || { echo 'Run natively on an Apple Silicon Mac.' >&2; exit 1; }
[[ $# == 1 && $1 == /* && ! -e $1 ]] || { echo 'Supply a new absolute output directory.' >&2; exit 1; }
for tool in curl shasum tar xz make xcrun python3 zip otool; do command -v "$tool" >/dev/null; done
script=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
out=$1
mkdir -p "$out"
version=9.0.2
source_url=https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz
source_sha256=8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e
package=ffmpeg-9.0.2-sonojot-lgpl-macos-arm64
cd "$out"
curl --fail --location --proto '=https' --tlsv1.2 --output "ffmpeg-$version.tar.xz" "$source_url"
printf '%s  %s\n' "$source_sha256" "ffmpeg-$version.tar.xz" | shasum -a 256 -c -
tar -xJf "ffmpeg-$version.tar.xz"
src="$out/ffmpeg-$version"
mkdir build "$package" "$package/bin" "$package/notices" source-material
cp "$script" source-material/build_ffmpeg_lgpl_macos.sh
cp "ffmpeg-$version.tar.xz" source-material/
export MACOSX_DEPLOYMENT_TARGET=12.0
export SDKROOT
SDKROOT=$(xcrun --sdk macosx --show-sdk-path)
export SOURCE_DATE_EPOCH
SOURCE_DATE_EPOCH=$(python3 - "$src/RELEASE" <<'PY'
import os,sys
print(int(os.stat(sys.argv[1]).st_mtime))
PY
)
{
  sw_vers
  xcodebuild -version || true
  xcrun clang --version
  xcrun --sdk macosx --show-sdk-version
  make --version
  printf 'Source: %s\nSHA-256: %s\nSOURCE_DATE_EPOCH: %s\n' "$source_url" "$source_sha256" "$SOURCE_DATE_EPOCH"
  printf 'MACOSX_DEPLOYMENT_TARGET: %s\nSDKROOT: %s\n' "$MACOSX_DEPLOYMENT_TARGET" "$SDKROOT"
} > source-material/toolchain.txt

# The app extracts the first audio stream to 16-kHz mono pcm_s16le WAV.
# mov covers M4A/MP4/MOV; matroska covers MKV/WebM; asf covers WMA.
# Only native audio decoders are enabled. No external codec libraries or video decoders.
decoders=aac,aac_fixed,aac_latm,ac3,ac3_fixed,eac3,alac,amrnb,amrwb,dca,flac,gsm,gsm_ms,mp1,mp1float,mp2,mp2float,mp3,mp3float,mp3adu,mp3adufloat,mp3on4,mp3on4float,opus,vorbis,wmav1,wmav2,wmapro,wmalossless,wmavoice,truehd,mlp
# Explicit native codecs: wildcards also select GPL-only and AudioToolbox variants.
decoders+=,pcm_alaw,pcm_bluray,pcm_dvd,pcm_f16le,pcm_f24le,pcm_f32be,pcm_f32le,pcm_f64be,pcm_f64le,pcm_lxf,pcm_mulaw,pcm_s16be,pcm_s16be_planar,pcm_s16le,pcm_s16le_planar,pcm_s24be,pcm_s24daud,pcm_s24le,pcm_s24le_planar,pcm_s32be,pcm_s32le,pcm_s32le_planar,pcm_s64be,pcm_s64le,pcm_s8,pcm_s8_planar,pcm_sga,pcm_u16be,pcm_u16le,pcm_u24be,pcm_u24le,pcm_u32be,pcm_u32le,pcm_u8,pcm_vidc
decoders+=,adpcm_ima_qt,adpcm_ima_wav,adpcm_ms,adpcm_g722,adpcm_g726,adpcm_g726le,adpcm_ima_dk3,adpcm_ima_dk4,adpcm_ima_ws,adpcm_ima_smjpeg,adpcm_swf,adpcm_yamaha,adpcm_ct,adpcm_sbpro_2,adpcm_sbpro_3,adpcm_sbpro_4,adpcm_adx,adpcm_thp,adpcm_thp_le,adpcm_xa,adpcm_psx
flags=(
  --prefix=/sonojot-ffmpeg --arch=arm64 --target-os=darwin
  --cc=clang --pkg-config=false --disable-autodetect --disable-everything
  --disable-gpl --disable-nonfree --disable-version3
  --enable-static --disable-shared --enable-pthreads
  --disable-network --disable-doc --disable-debug --disable-ffplay --disable-ffprobe
  --disable-avdevice --disable-swscale
  --enable-ffmpeg --enable-avcodec --enable-avformat --enable-avfilter --enable-swresample
  --enable-protocol=file,pipe
  --enable-demuxer=aac,flac,mp3,mov,matroska,ogg,wav,asf,asf_o,aiff,caf,amr,mpegts,mpegps,avi
  "--enable-decoder=$decoders"
  --enable-parser=aac,aac_latm,ac3,dca,flac,mpegaudio,opus,vorbis,mlp
  --enable-encoder=pcm_s16le --enable-muxer=wav
  --enable-filter=abuffer,abuffersink,aformat,anull,aresample,atrim
  --extra-version=sonojot-lgpl --enable-stripping
)
printf '%q ' ./configure "${flags[@]}" > source-material/configure-command.txt
printf '\n' >> source-material/configure-command.txt
cd build
# Clean environment prevents caller-supplied codec/linker flags from changing the license.
env -u CFLAGS -u CXXFLAGS -u LDFLAGS -u CPPFLAGS -u PKG_CONFIG_PATH \
  "$src/configure" "${flags[@]}" 2>&1 | tee "$out/source-material/configure-output.txt"
grep -q 'License: LGPL version 2.1 or later' "$out/source-material/configure-output.txt"
if grep -E '^#define CONFIG_(GPL|NONFREE|VERSION3) 1' config.h; then
  echo 'Forbidden license option enabled.' >&2; exit 1
fi
# --pkg-config=false deliberately triggers this one detection warning.
# Reject every other missing/disabled component warning.
if grep -v '^WARNING: false not found, library detection may fail\.$' "$out/source-material/configure-output.txt" | grep -Ei 'WARNING:.*(disabled|not found|did not match)'; then
  echo 'A requested import component was not configured.' >&2; exit 1
fi
make -j "${FFMPEG_BUILD_JOBS:-4}" 2>&1 | tee "$out/source-material/build-log.txt"
cp ffmpeg "$out/$package/bin/ffmpeg"
cp config.h config_components.h ffbuild/config.mak ffbuild/config.log "$out/source-material/"
cd "$out"
cp "$src/COPYING.LGPLv2.1" "$package/LICENSE.txt"
cp "$src/LICENSE.md" "$package/notices/FFmpeg-LICENSE.md"
"$package/bin/ffmpeg" -version > source-material/ffmpeg-version.txt 2>&1
"$package/bin/ffmpeg" -buildconf > source-material/ffmpeg-buildconf.txt 2>&1
"$package/bin/ffmpeg" -L > source-material/ffmpeg-license.txt 2>&1
"$package/bin/ffmpeg" -hide_banner -decoders > source-material/decoders.txt 2>&1
"$package/bin/ffmpeg" -hide_banner -demuxers > source-material/demuxers.txt 2>&1
otool -L "$package/bin/ffmpeg" > source-material/dynamic-libraries.txt
# A static FFmpeg executable may still link Apple's system libraries.
python3 - source-material/dynamic-libraries.txt <<'PY'
import sys
for line in open(sys.argv[1]).read().splitlines()[1:]:
    path=line.strip().split(' (',1)[0]
    if not path.startswith(('/usr/lib/','/System/Library/')):
        raise SystemExit('Unexpected non-system dependency: '+path)
PY
python3 - <<'PY'
import wave
with wave.open('smoke-input.wav','wb') as w:
    w.setparams((2,2,48000,4800,'NONE','not compressed'))
    w.writeframes(b'\0'*19200)
PY
"$package/bin/ffmpeg" -hide_banner -loglevel error -nostdin -y -protocol_whitelist file,pipe \
  -i smoke-input.wav -map 0:a:0 -vn -ac 1 -ar 16000 -c:a pcm_s16le -f wav smoke-output.wav
python3 - <<'PY'
import wave
with wave.open('smoke-output.wav','rb') as w:
    assert (w.getnchannels(),w.getsampwidth(),w.getframerate())==(1,2,16000)
    assert w.getnframes()>0
PY
cat > "$package/NOTICE.txt" <<'EOF'
FFmpeg 9.0.2, copyright the FFmpeg developers.
This converter contains FFmpeg under LGPL-2.1-or-later. See LICENSE.txt and notices/.
SonoJot configured the unmodified upstream source for local audio import and compiled it
with static FFmpeg libraries. GPL, nonfree and version3 code and external codec libraries
are disabled. It runs as a separate executable; SonoJot does not link these libraries.
The corresponding-source archive includes the exact upstream source, this build script,
configure flags, toolchain record and build logs. Recipients may rebuild and replace the
converter, including using SonoJot's Locate converter option.
SOURCE.txt must name the actual published binary and source URLs before distribution.
EOF
printf 'UNPUBLISHED: publish %s-corresponding-source.tar.xz beside the binary and replace this line with both exact HTTPS URLs.\n' "$package" > "$package/SOURCE.txt"
printf 'No upstream source modifications; configure/build outputs are outside the source tree.\n' > source-material/changes.txt
cp source-material/{configure-command.txt,toolchain.txt,ffmpeg-buildconf.txt,dynamic-libraries.txt} "$package/notices/"
tar -cJf "$package-corresponding-source.tar.xz" source-material
# Candidate only: after signing and filling SOURCE.txt, recreate the ZIP and its hashes.
COPYFILE_DISABLE=1 zip -X -r "$package-candidate.zip" "$package"
shasum -a 256 "$package-candidate.zip" "$package-corresponding-source.tar.xz" "$package/bin/ffmpeg" > SHA256SUMS
wc -c "$package-candidate.zip" "$package/bin/ffmpeg" > BYTE-SIZES.txt
printf 'Candidate built in %s. Complete publication and codec smoke checks in docs/LicenseReview.md before pinning.\n' "$out"
