# WebP Studio

애니메이션 WebP 를 줄이고, 넘기기 전에 흔한 결함을 검사하는 macOS 데스크톱 앱입니다.

애니메이션 WebP 를 받아 쓰다 보면 비슷한 문제가 되풀이됩니다. 파일 하나가 수 MB 이거나,
애니메이션이 아니라 프레임 1장짜리 정지 이미지로 내보내져 있거나, 맨 아래 1px 줄이 검게 나오는 식입니다.
그때마다 만든 사람에게 다시 부탁하면 하루씩 밀립니다. 이 앱은 크기·fps·용량 조정과 아래쪽 줄 보정을 바로 처리하고,
그림 자체를 다시 만들어야 하는 문제만 걸러 냅니다.

## 하는 일

- **입력**: 애니메이션 WebP, 영상(mp4·mov·m4v·webm·gif), PNG 시퀀스 폴더. 끌어다 놓거나 버튼으로 추가합니다.
- **변환**: 폭 줄이기(Lanczos3), fps 낮추기, 손실 압축 품질 조정, 아래쪽 1px 줄 자동 보정
- **출력**: `<이름>.webp`(애니메이션) + `<이름>.png`(첫 프레임 정지본) + `report.json`(검사 결과)
- **검사**: 받은 파일과 변환본을 모두 검사해 칩으로 보여 줍니다. 칩에 마우스를 올리면 이유가 나옵니다.

| 검사 | 통과 기준 | 실패하면 |
|---|---|---|
| 애니메이션 | 프레임 수 > 1 | 정지 이미지로 내보낸 파일. 다시 내보내야 합니다 |
| 아래쪽 줄 | 맨 아래 행과 바로 위 행의 평균 밝기 차 ≤ 48 | 변환할 때 자동 보정됩니다 |
| 용량 | 상한 이하(기본 1.5MB) | 폭·fps·품질을 낮춥니다 |
| 정지본 비율 | PNG 와 WebP 가로세로 비율 차 < 1% | 변환하면 같은 프레임에서 정지본을 만듭니다 |
| 해상도 | 폭 ≤ 화면 표시 폭 × 3 | 출력 폭을 줄입니다 |

### 프리셋

| 프리셋 | 폭 | fps | 품질 |
|---|---|---|---|
| 모바일 웹 카드 (표시 폭 380px) | 760px | 20 | 80 |
| 3배율 폰까지 선명하게 | 1000px | 24 | 80 |
| 크기·fps 유지, 압축만 | 원본 | 원본 | 80 |

실측 예: 1200×888·30fps·151프레임 3.30MB WebP → "모바일 웹 카드" 프리셋으로 760×562·20fps·101프레임 1.23MB (Apple M 시리즈에서 약 1.6초).

## 설치

Releases 의 `WebP Studio_<버전>_aarch64.dmg` 를 열어 앱을 Applications 로 옮깁니다. Apple Silicon Mac 전용입니다.

서명하지 않은 앱이라 처음에는 macOS 가 막습니다. Finder 에서 앱을 **우클릭 → 열기**를 한 번 하면 그다음부터는 그냥 열립니다.

## 개발

필요한 것: Rust(stable), Node 20+, pnpm, Apple Silicon Mac.

```bash
pnpm install
pnpm tauri:dev     # ffmpeg 를 받고 개발 모드로 실행
pnpm tauri:build   # .app 과 .dmg 생성 (src-tauri/target/release/bundle/)
cd src-tauri && cargo test
```

`tauri build` 를 직접 부를 때는 `CI=true` 를 붙여야 합니다. 붙이지 않으면 dmg 꾸미기 단계에서 Finder AppleScript 가 시간 초과로 실패할 수 있습니다.

실행 인자로 파일을 넘기면 열자마자 목록에 올라갑니다.

```bash
open -n "WebP Studio.app" --args ~/Desktop/a.webp ~/Desktop/b.mov
```

### 구조

```
src/                 React 화면 (App.tsx, api.ts)
src-tauri/src/
  source.rs          입력을 프레임 단위로 읽기 (WebP·영상·PNG 폴더)
  process.rs         검사(inspect)·변환(convert) 파이프라인
  checks.rs          검사 기준과 아래쪽 줄 보정
  ffmpeg.rs          영상 입력용 ffmpeg 호출
  commands.rs        Tauri command
scripts/fetch-ffmpeg.sh
```

- WebP 해독·인코딩은 libwebp(`webp-animation` crate, 정적 링크)로 직접 합니다. ffmpeg 8.x 는 애니메이션 WebP 를 읽지 못하기 때문입니다.
- 프레임을 한꺼번에 메모리에 올리지 않고 한 장씩 흘려보냅니다(1200×888 에 268프레임이면 RGBA 로 1GB 가 넘습니다).
- 영상은 ffmpeg 로 임시 폴더에 PNG 를 뽑은 뒤 같은 파이프라인으로 처리합니다.

### 실제 파일로 테스트

공개 레포라 실제 자산은 커밋하지 않습니다. `fixtures/local/`(git 제외)에 `current.webp`(정상 애니메이션)와
`broken-still.webp`(프레임 1장 + 검은 줄)를 넣으면 `cargo test` 가 그 파일로도 검사합니다. 없으면 건너뜁니다.

## 라이선스

앱 코드는 MIT 입니다.

영상 입력용으로 함께 묶는 ffmpeg 는 [martin-riedl.de](https://ffmpeg.martin-riedl.de/) 의 정적 빌드(FFmpeg 9.0.2)이며 GPL 로 배포됩니다.
바이너리는 레포에 포함하지 않고 `scripts/fetch-ffmpeg.sh` 가 고정 버전을 받아 sha256 을 확인합니다.
FFmpeg 소스는 <https://ffmpeg.org/download.html> 에서 받을 수 있습니다.
