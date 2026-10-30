//! 입력 파일을 프레임 단위로 읽는다.
//!
//! 애니메이션 WebP 한 장이 1200×888 에 268프레임이면 RGBA 로 1GB 가 넘는다. 그래서 프레임을
//! 한꺼번에 메모리에 올리지 않고 [`FrameSource`] 로 한 장씩 꺼내 처리한다.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::ffmpeg;

/// 한 프레임. `rgba` 는 `width * height * 4` 바이트다.
pub struct Frame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// 이 프레임이 화면에 나타나는 시각(ms)
    pub start_ms: i32,
    /// 이 프레임이 화면에 머무는 시간(ms)
    pub duration_ms: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceKind {
    Webp,
    Video,
    PngSequence,
}

pub fn detect_kind(path: &Path) -> Result<SourceKind> {
    if path.is_dir() {
        return Ok(SourceKind::PngSequence);
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "webp" => Ok(SourceKind::Webp),
        "mp4" | "mov" | "m4v" | "webm" | "gif" => Ok(SourceKind::Video),
        _ => bail!("지원하지 않는 파일 형식입니다: .{ext} (WebP, mp4, mov, PNG 폴더만 받습니다)"),
    }
}

/// 프레임을 하나씩 내주는 입력.
pub trait FrameSource: Send {
    fn next_frame(&mut self) -> Result<Option<Frame>>;
}

/// 애니메이션(또는 정지) WebP.
///
/// libwebp 의 `WebPAnimDecoder` 가 부분 프레임을 합성해 매 프레임 전체 화면을 돌려준다.
/// `WebPAnimDecoderGetNext` 가 주는 시각은 프레임이 **끝나는** 시각이라 시작 시각은 직전 값으로 잡는다.
///
/// 디코더가 입력 버퍼를 빌리므로, 버퍼를 힙에 고정해 두고 디코더를 먼저 버린 뒤 버퍼를 해제한다.
/// 이렇게 해야 프레임을 모으지 않고 한 장씩 꺼낼 수 있다.
pub struct WebpSource {
    iter: Option<webp_animation::DecoderIterator<'static>>,
    bytes: *mut [u8],
    prev_end_ms: i32,
}

// 버퍼는 이 값만 소유하고 다른 스레드와 공유하지 않는다.
unsafe impl Send for WebpSource {}

impl WebpSource {
    pub fn open(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("파일을 읽지 못했습니다: {}", path.display()))?;
        Self::from_bytes(bytes)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        let raw: *mut [u8] = Box::into_raw(bytes.into_boxed_slice());
        // SAFETY: `raw` 는 Drop 에서 디코더를 먼저 버린 뒤에만 해제한다.
        let slice: &'static [u8] = unsafe { &*raw };
        match webp_animation::Decoder::new(slice) {
            Ok(decoder) => Ok(Self { iter: Some(decoder.into_iter()), bytes: raw, prev_end_ms: 0 }),
            Err(e) => {
                unsafe { drop(Box::from_raw(raw)) };
                Err(anyhow::anyhow!("WebP 해독 실패: {e:?}"))
            }
        }
    }
}

impl Drop for WebpSource {
    fn drop(&mut self) {
        self.iter.take();
        // SAFETY: 디코더가 사라졌으므로 버퍼를 빌린 곳이 없다.
        unsafe { drop(Box::from_raw(self.bytes)) };
    }
}

impl FrameSource for WebpSource {
    fn next_frame(&mut self) -> Result<Option<Frame>> {
        let Some(frame) = self.iter.as_mut().and_then(|it| it.next()) else { return Ok(None) };
        let (width, height) = frame.dimensions();
        let end = frame.timestamp();
        let start = self.prev_end_ms;
        self.prev_end_ms = end;
        Ok(Some(Frame {
            rgba: frame.data().to_vec(),
            width,
            height,
            start_ms: start,
            duration_ms: (end - start).max(1),
        }))
    }
}

/// PNG 파일 목록(폴더 안 PNG 를 이름순으로)을 주어진 fps 로 재생되는 프레임으로 읽는다.
pub struct PngSequenceSource {
    paths: std::vec::IntoIter<PathBuf>,
    frame_ms: f64,
    index: usize,
}

impl PngSequenceSource {
    pub fn open_dir(dir: &Path, fps: f64) -> Result<Self> {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
            .with_context(|| format!("폴더를 읽지 못했습니다: {}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("png")))
            .collect();
        if paths.is_empty() {
            bail!("폴더에 PNG 파일이 없습니다: {}", dir.display());
        }
        paths.sort();
        Ok(Self::from_paths(paths, fps))
    }

    pub fn from_paths(paths: Vec<PathBuf>, fps: f64) -> Self {
        Self { paths: paths.into_iter(), frame_ms: 1000.0 / fps, index: 0 }
    }
}

impl FrameSource for PngSequenceSource {
    fn next_frame(&mut self) -> Result<Option<Frame>> {
        let Some(path) = self.paths.next() else { return Ok(None) };
        let img = image::open(&path)
            .with_context(|| format!("PNG 를 읽지 못했습니다: {}", path.display()))?
            .into_rgba8();
        let (width, height) = img.dimensions();
        let start = (self.index as f64 * self.frame_ms).round() as i32;
        let end = ((self.index + 1) as f64 * self.frame_ms).round() as i32;
        self.index += 1;
        Ok(Some(Frame { rgba: img.into_raw(), width, height, start_ms: start, duration_ms: end - start }))
    }
}

/// 영상은 ffmpeg 로 임시 폴더에 PNG 를 뽑은 뒤 PNG 시퀀스로 읽는다. 임시 폴더는 이 값이 사라질 때 지워진다.
pub struct VideoSource {
    inner: PngSequenceSource,
    _dir: tempfile::TempDir,
}

impl VideoSource {
    /// `target_fps` 가 있으면 ffmpeg 단계에서 미리 그 fps 로 뽑아 PNG 수를 줄인다.
    pub fn open(path: &Path, target_fps: Option<f64>) -> Result<Self> {
        let info = ffmpeg::probe(path)?;
        let fps = match target_fps {
            Some(t) if t < info.fps => t,
            _ => info.fps,
        };
        let dir = tempfile::tempdir()?;
        ffmpeg::extract_frames(path, dir.path(), (fps < info.fps).then_some(fps))?;
        let inner = PngSequenceSource::open_dir(dir.path(), fps)?;
        Ok(Self { inner, _dir: dir })
    }
}

impl FrameSource for VideoSource {
    fn next_frame(&mut self) -> Result<Option<Frame>> {
        self.inner.next_frame()
    }
}

pub fn open_source(path: &Path, png_fps: f64, target_fps: Option<f64>) -> Result<Box<dyn FrameSource>> {
    Ok(match detect_kind(path)? {
        SourceKind::Webp => Box::new(WebpSource::open(path)?),
        SourceKind::Video => Box::new(VideoSource::open(path, target_fps)?),
        SourceKind::PngSequence => Box::new(PngSequenceSource::open_dir(path, png_fps)?),
    })
}
