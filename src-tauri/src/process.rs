//! 검사(inspect)와 변환(convert). 둘 다 프레임을 한 장씩 흘려보내며 처리한다.

use std::path::Path;

use anyhow::{bail, Context, Result};
use fast_image_resize::images::Image;
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use serde::{Deserialize, Serialize};
use webp_animation::prelude::*;

use crate::checks::{self, Check, Facts, Limits};
use crate::source::{self, detect_kind, Frame, SourceKind};

/// PNG 시퀀스 폴더는 fps 정보가 없으므로 따로 받는다.
pub const DEFAULT_SEQUENCE_FPS: f64 = 30.0;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inspection {
    pub kind: SourceKind,
    pub facts: Facts,
    pub fps: f64,
    pub checks: Vec<Check>,
}

fn input_bytes(path: &Path) -> u64 {
    if path.is_dir() {
        std::fs::read_dir(path)
            .map(|it| it.filter_map(|e| e.ok()?.metadata().ok()).map(|m| m.len()).sum())
            .unwrap_or(0)
    } else {
        std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    }
}

fn png_size(path: &Path) -> Option<(u32, u32)> {
    image::image_dimensions(path).ok()
}

/// 파일을 끝까지 읽으며 검사에 필요한 사실을 모은다.
pub fn inspect(path: &Path, poster: Option<&Path>, limits: &Limits, sequence_fps: f64) -> Result<Inspection> {
    let kind = detect_kind(path)?;
    let mut src = source::open_source(path, sequence_fps, None)?;
    let mut facts = Facts { bytes: input_bytes(path), poster_size: poster.and_then(png_size), ..Default::default() };

    while let Some(frame) = src.next_frame()? {
        if facts.frame_count == 0 {
            facts.width = frame.width;
            facts.height = frame.height;
        }
        facts.frame_count += 1;
        facts.duration_ms = frame.start_ms + frame.duration_ms;
        if checks::has_bottom_row_defect(&frame.rgba, frame.width, frame.height) {
            facts.bottom_row_defect_frames += 1;
        }
    }
    if facts.frame_count == 0 {
        bail!("프레임을 하나도 읽지 못했습니다: {}", path.display());
    }

    let checks = checks::run(&facts, limits);
    Ok(Inspection { kind, fps: facts.fps(), facts, checks })
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertOptions {
    /// 출력 폭(px). 원본보다 크면 키우지 않는다. `None` 이면 원본 폭.
    pub width: Option<u32>,
    /// 출력 fps. 원본보다 높으면 원본을 그대로 둔다. `None` 이면 원본 fps.
    pub fps: Option<f64>,
    /// libwebp 손실 압축 품질(0~100)
    pub quality: f32,
    pub fix_bottom_row: bool,
    /// PNG 시퀀스 입력의 fps
    pub sequence_fps: f64,
}

impl Default for ConvertOptions {
    /// 모바일 웹 카드(표시 폭 335~380px) 기준. 1200×888·30fps 3.30MB 파일이 1.23MB 로 줄었던 설정이다.
    fn default() -> Self {
        Self { width: Some(760), fps: Some(20.0), quality: 80.0, fix_bottom_row: true, sequence_fps: DEFAULT_SEQUENCE_FPS }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertOutcome {
    pub frames_in: usize,
    pub frames_out: usize,
    /// 아래쪽 줄을 보정한 프레임 수
    pub fixed_bottom_rows: usize,
    pub output: Inspection,
}

/// 출력 시각을 고른다. 목표 fps 가 원본보다 낮을 때만 프레임을 솎아 낸다.
///
/// 출력 칸(0, step, 2·step …)마다 그 시각을 덮는 입력 프레임 하나를 고른다. 한 입력 프레임이
/// 여러 칸을 덮으면(원본이 느린 구간) 한 번만 내보내고 다음 출력까지 그대로 머문다.
struct FrameSampler {
    step_ms: Option<f64>,
    next_slot_ms: f64,
}

impl FrameSampler {
    fn new(fps: Option<f64>) -> Self {
        Self { step_ms: fps.filter(|f| *f > 0.0).map(|f| 1000.0 / f), next_slot_ms: 0.0 }
    }

    /// 이 프레임을 내보내야 하면 출력 시각을 돌려준다.
    fn take(&mut self, start_ms: i32, duration_ms: i32) -> Option<i32> {
        let Some(step) = self.step_ms else { return Some(start_ms) };
        let end = (start_ms + duration_ms) as f64;
        if self.next_slot_ms >= end {
            return None;
        }
        let at = self.next_slot_ms.max(start_ms as f64);
        while self.next_slot_ms < end {
            self.next_slot_ms += step;
        }
        Some(at.round() as i32)
    }
}

fn resize_frame(resizer: &mut Resizer, frame: &Frame, width: u32) -> Result<(Vec<u8>, u32, u32)> {
    let height = ((frame.height as f64 * width as f64 / frame.width as f64).round() as u32).max(1);
    let src = Image::from_vec_u8(frame.width, frame.height, frame.rgba.clone(), PixelType::U8x4)?;
    let mut dst = Image::new(width, height, PixelType::U8x4);
    let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3));
    resizer.resize(&src, &mut dst, &options)?;
    Ok((dst.into_vec(), width, height))
}

/// 변환해서 `out_webp`(애니메이션)와 `out_poster`(첫 프레임 PNG)를 쓴다.
/// `on_progress(처리한 입력 프레임 수)` 는 프레임마다 불린다.
pub fn convert(
    path: &Path,
    options: &ConvertOptions,
    limits: &Limits,
    out_webp: &Path,
    out_poster: &Path,
    mut on_progress: impl FnMut(usize),
) -> Result<ConvertOutcome> {
    let mut src = source::open_source(path, options.sequence_fps, options.fps)?;
    let mut sampler = FrameSampler::new(options.fps);
    let mut resizer = Resizer::new();
    let mut encoder: Option<Encoder> = None;
    let (mut frames_in, mut frames_out, mut fixed) = (0usize, 0usize, 0usize);
    let mut end_ms = 0;

    while let Some(mut frame) = src.next_frame()? {
        frames_in += 1;
        end_ms = frame.start_ms + frame.duration_ms;

        let Some(timestamp) = sampler.take(frame.start_ms, frame.duration_ms) else {
            on_progress(frames_in);
            continue;
        };

        if options.fix_bottom_row && checks::has_bottom_row_defect(&frame.rgba, frame.width, frame.height) {
            checks::fix_bottom_row(&mut frame.rgba, frame.width, frame.height);
            fixed += 1;
        }

        let (rgba, width, height) = match options.width {
            Some(w) if w < frame.width => resize_frame(&mut resizer, &frame, w)?,
            _ => (std::mem::take(&mut frame.rgba), frame.width, frame.height),
        };

        if encoder.is_none() {
            image::save_buffer(out_poster, &rgba, width, height, image::ExtendedColorType::Rgba8)
                .with_context(|| format!("정지본 PNG 를 쓰지 못했습니다: {}", out_poster.display()))?;
            let enc = Encoder::new_with_options(
                (width, height),
                EncoderOptions {
                    encoding_config: Some(EncodingConfig::new_lossy(options.quality)),
                    ..Default::default()
                },
            )
            .map_err(|e| anyhow::anyhow!("인코더를 만들지 못했습니다: {e:?}"))?;
            encoder = Some(enc);
        }
        encoder
            .as_mut()
            .unwrap()
            .add_frame(&rgba, timestamp)
            .map_err(|e| anyhow::anyhow!("{frames_in}번째 프레임 인코딩 실패: {e:?}"))?;
        frames_out += 1;
        on_progress(frames_in);
    }
    let Some(encoder) = encoder else { bail!("프레임을 하나도 읽지 못했습니다: {}", path.display()) };
    let data = encoder.finalize(end_ms).map_err(|e| anyhow::anyhow!("WebP 마무리 실패: {e:?}"))?;
    std::fs::write(out_webp, &*data).with_context(|| format!("WebP 를 쓰지 못했습니다: {}", out_webp.display()))?;

    let output = inspect(out_webp, Some(out_poster), limits, options.sequence_fps)?;
    Ok(ConvertOutcome { frames_in, frames_out, fixed_bottom_rows: fixed, output })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::{CheckId, Status};

    /// 움직이는 사각형 애니메이션(30fps, 30프레임)을 만든다. 마지막 행을 검게 칠할 수 있다.
    pub(crate) fn synthetic_webp(width: u32, height: u32, frames: i32, black_bottom: bool) -> Vec<u8> {
        let mut enc = Encoder::new_with_options(
            (width, height),
            EncoderOptions { encoding_config: Some(EncodingConfig::new_lossy(90.0)), ..Default::default() },
        )
        .unwrap();
        for i in 0..frames {
            let mut rgba = [240u8, 244, 248, 255].repeat((width * height) as usize);
            let x0 = (i as u32 * 4) % (width - 16);
            for y in 8..24.min(height) {
                for x in x0..x0 + 16 {
                    let p = ((y * width + x) * 4) as usize;
                    rgba[p..p + 3].copy_from_slice(&[15, 123, 131]);
                }
            }
            if black_bottom {
                let last = ((height - 1) * width * 4) as usize;
                rgba[last..].chunks_exact_mut(4).for_each(|p| p[..3].fill(0));
            }
            enc.add_frame(&rgba, i * 33).unwrap();
        }
        enc.finalize(frames * 33).unwrap().to_vec()
    }

    fn status(checks: &[Check], id: CheckId) -> Status {
        checks.iter().find(|c| c.id == id).unwrap().status
    }

    #[test]
    fn sampler_halves_30_to_15fps() {
        let mut s = FrameSampler::new(Some(15.0));
        let kept: Vec<_> = (0..30).filter_map(|i| s.take(i * 33, 33)).collect();
        assert_eq!(kept.len(), 15);
    }

    #[test]
    fn sampler_keeps_everything_without_target() {
        let mut s = FrameSampler::new(None);
        assert_eq!((0..10).filter_map(|i| s.take(i * 33, 33)).count(), 10);
    }

    #[test]
    fn inspect_flags_black_bottom_row_and_convert_fixes_it() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.webp");
        std::fs::write(&input, synthetic_webp(320, 240, 30, true)).unwrap();

        let limits = Limits::default();
        let before = inspect(&input, None, &limits, 30.0).unwrap();
        assert_eq!(before.facts.frame_count, 30);
        assert_eq!(status(&before.checks, CheckId::BottomRow), Status::Fail);

        let (out, poster) = (dir.path().join("out.webp"), dir.path().join("out.png"));
        let opts = ConvertOptions { width: Some(160), fps: Some(15.0), ..Default::default() };
        let outcome = convert(&input, &opts, &limits, &out, &poster, |_| {}).unwrap();

        assert_eq!(outcome.frames_out, 15);
        assert!(outcome.fixed_bottom_rows > 0);
        let after = outcome.output;
        assert_eq!((after.facts.width, after.facts.height), (160, 120));
        assert_eq!(status(&after.checks, CheckId::BottomRow), Status::Pass);
        assert_eq!(status(&after.checks, CheckId::Animated), Status::Pass);
        assert_eq!(status(&after.checks, CheckId::PosterRatio), Status::Pass);
    }

    #[test]
    fn single_frame_webp_fails_animated_check() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("still.webp");
        std::fs::write(&input, synthetic_webp(64, 48, 1, false)).unwrap();
        let result = inspect(&input, None, &Limits::default(), 30.0).unwrap();
        assert_eq!(status(&result.checks, CheckId::Animated), Status::Fail);
    }

    /// `fixtures/local/` 에 실제 파일이 있을 때만 돈다(공개 레포라 커밋하지 않음).
    #[test]
    fn real_files_when_available() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/local");
        let current = dir.join("current.webp");
        if !current.exists() {
            eprintln!("fixtures/local/current.webp 없음 — 건너뜀");
            return;
        }
        let limits = Limits::default();
        let before = inspect(&current, None, &limits, 30.0).unwrap();
        eprintln!("before: {:?}", before.facts);

        let tmp = tempfile::tempdir().unwrap();
        let (out, poster) = (tmp.path().join("o.webp"), tmp.path().join("o.png"));
        let started = std::time::Instant::now();
        let outcome = convert(&current, &ConvertOptions::default(), &limits, &out, &poster, |_| {}).unwrap();
        eprintln!("after: {:?} in {:?}", outcome.output.facts, started.elapsed());
        assert!(outcome.output.facts.frame_count > 1);
        assert!(outcome.output.facts.bytes < before.facts.bytes);

        let broken = dir.join("broken-still.webp");
        if broken.exists() {
            let r = inspect(&broken, None, &limits, 30.0).unwrap();
            assert_eq!(status(&r.checks, CheckId::Animated), Status::Fail);
            assert_eq!(status(&r.checks, CheckId::BottomRow), Status::Fail);
        }
    }
}
