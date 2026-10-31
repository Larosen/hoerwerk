#!/bin/sh
# Builds a minimal FFmpeg for Hörwerk: only the audio formats it reads, AAC/M4B output,
# no network code at all, no external libraries, hardened toolchain.
#
#   build-ffmpeg.sh <source dir> <install prefix>
#
# Everything not listed here (video codecs, streaming protocols, HLS/DASH, devices, filters
# beyond audio resampling …) is simply not in the binary – most FFmpeg CVEs concern exactly
# those parts.
set -eu
SRC=$1
PREFIX=$2
JOBS=${JOBS:-$(nproc)}

# Audio inputs (keep in sync with AUDIO_DEMUXERS in app/meta.py), plus the video containers the
# renamer reads for {resolution}/{codec}/{fps} (VIDEO_DEMUXERS) – containers only, no video decoders.
DEMUXERS="mp3,mov,aac,flac,ogg,wav,w64,aiff,asf,matroska,ape,wv,caf,mpc,mpc8,tta,au,amr,dsf,ffmetadata,\
avi,mpegts,mpegps,flv"
DECODERS="mp3float,mp3,mp2float,mp2,mp1float,mp1,aac,aac_fixed,aac_latm,alac,flac,vorbis,opus,\
ac3,eac3,wmav1,wmav2,wmapro,wmalossless,ape,wavpack,mpc7,mpc8,tta,amrnb,amrwb,\
dsd_lsbf,dsd_msbf,dsd_lsbf_planar,dsd_msbf_planar,\
pcm_s8,pcm_u8,pcm_s16le,pcm_s16be,pcm_s24le,pcm_s24be,pcm_s32le,pcm_s32be,pcm_f32le,pcm_f32be,\
pcm_f64le,pcm_f64be,pcm_alaw,pcm_mulaw"
PARSERS="aac,aac_latm,mpegaudio,flac,vorbis,opus,ac3,dca,mjpeg,png,h264,hevc,mpeg4video,mpegvideo,vc1,vp8,vp9,av1"
# M4B output, ADTS segments, embedded cover pictures (copied, never decoded)
ENCODERS="aac"
MUXERS="adts,mp4,ipod,image2"
BSFS="aac_adtstoasc,extract_extradata"
FILTERS="aformat,aresample,anull,atrim,null,format"   # (a)buffer(sink) are always built in
PROTOCOLS="file,concat"

cd "$SRC"
# configure silently ignores unknown names – fail loudly instead
check() {
  kind=$1
  list=$(./configure --list-"$kind" | tr -s ' \t' '\n')
  for name in $(echo "$2" | tr ',' ' '); do
    echo "$list" | grep -qx "$name" || { echo "build-ffmpeg: unbekannt in $kind: $name" >&2; exit 1; }
  done
}
check demuxers "$DEMUXERS"; check decoders "$DECODERS"; check parsers "$PARSERS"; check encoders "$ENCODERS"
check muxers "$MUXERS"; check bsfs "$BSFS"; check filters "$FILTERS"; check protocols "$PROTOCOLS"

./configure \
  --prefix="$PREFIX" \
  --toolchain=hardened \
  --disable-debug --disable-doc --disable-htmlpages --disable-manpages --disable-podpages --disable-txtpages \
  --disable-autodetect --disable-network --disable-everything \
  --disable-avdevice --disable-swscale --disable-ffplay \
  --enable-ffmpeg --enable-ffprobe \
  --disable-shared --enable-static \
  --enable-demuxer="$DEMUXERS" --enable-decoder="$DECODERS" --enable-parser="$PARSERS" \
  --enable-encoder="$ENCODERS" --enable-muxer="$MUXERS" --enable-bsf="$BSFS" \
  --enable-filter="$FILTERS" --enable-protocol="$PROTOCOLS" \
  --enable-swresample \
  --extra-version=hoerwerk
make -j"$JOBS"
make install
"$PREFIX/bin/ffmpeg" -hide_banner -version | head -1
