#!/usr/bin/env bash
# 영상(mp4·mov) 입력용 ffmpeg 정적 빌드를 받아 Tauri sidecar 자리에 둔다.
#
# Homebrew ffmpeg 는 동적 라이브러리에 의존해 앱에 묶을 수 없어서, 시스템 프레임워크만 쓰는
# 정적 빌드(martin-riedl.de, FFmpeg 9.0.2 release)를 고정 버전으로 받는다.
# 바이너리는 GPL 이라 레포에 커밋하지 않는다(.gitignore). 다시 받으려면 파일을 지우고 재실행한다.
set -euo pipefail

VERSION_PATH="1789931890_9.0.2"
SHA256="c8ed4c4e6978a03c485edbfe4e0a5dc2380f8a30bba5150531b31b094492d924"
URL="https://ffmpeg.martin-riedl.de/download/macos/arm64/${VERSION_PATH}/ffmpeg.zip"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
DEST="${ROOT}/src-tauri/binaries/ffmpeg-${TRIPLE}"

if [[ "${TRIPLE}" != "aarch64-apple-darwin" ]]; then
  echo "지금은 Apple Silicon(aarch64-apple-darwin)만 지원합니다: ${TRIPLE}" >&2
  exit 1
fi

if [[ -x "${DEST}" ]]; then
  echo "이미 있습니다: ${DEST}"
  exit 0
fi

TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT

echo "ffmpeg 받는 중: ${URL}"
curl -fsSL -o "${TMP}/ffmpeg.zip" "${URL}"
echo "${SHA256}  ${TMP}/ffmpeg.zip" | shasum -a 256 -c - >/dev/null
unzip -q "${TMP}/ffmpeg.zip" -d "${TMP}"

mkdir -p "$(dirname "${DEST}")"
mv "${TMP}/ffmpeg" "${DEST}"
chmod +x "${DEST}"
xattr -d com.apple.quarantine "${DEST}" 2>/dev/null || true
echo "완료: ${DEST}"
