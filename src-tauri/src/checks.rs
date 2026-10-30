//! 넘기기 전에 걸러야 할 결함을 검사한다.
//!
//! 기준은 실제로 파일을 다시 받느라 하루씩 밀렸던 문제에서 왔다.
//! - 프레임 1장짜리 정지 이미지로 내보낸 파일 → [`CheckId::Animated`]
//! - 맨 아래 1px 이 검은 줄 → [`CheckId::BottomRow`]
//! - 장당 3~5.7MB → [`CheckId::FileSize`]
//! - 정지 PNG 와 WebP 비율 불일치 → [`CheckId::PosterRatio`]
//! - 화면 폭의 3배가 넘는 해상도 → [`CheckId::Resolution`]

use serde::Serialize;

/// 마지막 행과 바로 위 행의 평균 밝기 차가 이보다 크면 줄로 본다.
/// 실제 결함은 255 → 0 이었고, 그림 속 가로선은 대개 1px 보다 두꺼워 위 행과 함께 바뀐다.
pub const BOTTOM_ROW_LUMA_GAP: f64 = 48.0;

/// 행 하나의 평균 밝기(Rec.601, 0~255).
pub fn row_luma(rgba: &[u8], width: u32, row: u32) -> f64 {
    let stride = width as usize * 4;
    let start = row as usize * stride;
    let line = &rgba[start..start + stride];
    let sum: f64 = line
        .chunks_exact(4)
        .map(|p| 0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64)
        .sum();
    sum / width as f64
}

/// 마지막 행이 위 행과 동떨어져 있으면 그 밝기 차를 돌려준다.
pub fn bottom_row_gap(rgba: &[u8], width: u32, height: u32) -> f64 {
    if height < 2 {
        return 0.0;
    }
    (row_luma(rgba, width, height - 1) - row_luma(rgba, width, height - 2)).abs()
}

pub fn has_bottom_row_defect(rgba: &[u8], width: u32, height: u32) -> bool {
    bottom_row_gap(rgba, width, height) > BOTTOM_ROW_LUMA_GAP
}

/// 마지막 행을 바로 위 행으로 덮어쓴다. 해상도와 비율은 그대로다.
pub fn fix_bottom_row(rgba: &mut [u8], width: u32, height: u32) {
    if height < 2 {
        return;
    }
    let stride = width as usize * 4;
    let last = (height as usize - 1) * stride;
    rgba.copy_within(last - stride..last, last);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckId {
    Animated,
    BottomRow,
    FileSize,
    PosterRatio,
    Resolution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Pass,
    Fail,
    /// 판단할 재료가 없음(예: 정지본 PNG 가 아직 없음)
    Skip,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub id: CheckId,
    pub status: Status,
    /// 화면에 그대로 보여 줄 한 줄 설명
    pub detail: String,
}

#[derive(Debug, Clone, Copy, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    pub max_bytes: u64,
    /// 화면에 실제로 그려지는 최대 폭(px). 해상도 검사 기준은 이 값의 3배다.
    pub display_width: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_bytes: 1_500_000, display_width: 380 }
    }
}

/// 검사에 필요한 사실만 모은 값. 프레임 전체를 들고 있지 않는다.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Facts {
    pub width: u32,
    pub height: u32,
    pub frame_count: usize,
    pub duration_ms: i32,
    pub bytes: u64,
    /// 줄이 보인 프레임 수
    pub bottom_row_defect_frames: usize,
    /// 함께 쓰는 정지본 PNG 크기(있을 때만)
    pub poster_size: Option<(u32, u32)>,
}

impl Facts {
    pub fn fps(&self) -> f64 {
        if self.duration_ms <= 0 || self.frame_count < 2 {
            return 0.0;
        }
        self.frame_count as f64 * 1000.0 / self.duration_ms as f64
    }
}

fn mb(bytes: u64) -> String {
    format!("{:.2}MB", bytes as f64 / 1_000_000.0)
}

pub fn run(facts: &Facts, limits: &Limits) -> Vec<Check> {
    let animated = if facts.frame_count > 1 {
        Check { id: CheckId::Animated, status: Status::Pass, detail: format!("{}프레임", facts.frame_count) }
    } else {
        Check {
            id: CheckId::Animated,
            status: Status::Fail,
            detail: "프레임이 1장뿐인 정지 이미지입니다. 애니메이션으로 다시 내보내야 합니다.".into(),
        }
    };

    let bottom_row = if facts.bottom_row_defect_frames == 0 {
        Check { id: CheckId::BottomRow, status: Status::Pass, detail: "맨 아래 줄 정상".into() }
    } else {
        Check {
            id: CheckId::BottomRow,
            status: Status::Fail,
            detail: format!("맨 아래 1px 줄이 튀는 프레임 {}개 (변환하면 자동 보정)", facts.bottom_row_defect_frames),
        }
    };

    let file_size = Check {
        id: CheckId::FileSize,
        status: if facts.bytes <= limits.max_bytes { Status::Pass } else { Status::Fail },
        detail: format!("{} / 상한 {}", mb(facts.bytes), mb(limits.max_bytes)),
    };

    let poster_ratio = match facts.poster_size {
        None => Check { id: CheckId::PosterRatio, status: Status::Skip, detail: "정지본 PNG 없음".into() },
        Some((pw, ph)) => {
            let a = facts.width as f64 / facts.height as f64;
            let b = pw as f64 / ph as f64;
            let ok = (a - b).abs() / a < 0.01;
            Check {
                id: CheckId::PosterRatio,
                status: if ok { Status::Pass } else { Status::Fail },
                detail: format!("WebP {}×{} · PNG {pw}×{ph}", facts.width, facts.height),
            }
        }
    };

    let max_width = limits.display_width * 3;
    let resolution = Check {
        id: CheckId::Resolution,
        status: if facts.width <= max_width { Status::Pass } else { Status::Fail },
        detail: format!("폭 {}px / 상한 {max_width}px (표시 폭 {}px × 3)", facts.width, limits.display_width),
    };

    vec![animated, bottom_row, file_size, poster_ratio, resolution]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, value: u8) -> Vec<u8> {
        [value, value, value, 255].repeat((width * height) as usize)
    }

    #[test]
    fn detects_and_fixes_black_bottom_row() {
        let (w, h) = (8, 6);
        let mut img = solid(w, h, 255);
        let last = ((h - 1) * w * 4) as usize;
        img[last..].chunks_exact_mut(4).for_each(|p| p[..3].fill(0));

        assert!(has_bottom_row_defect(&img, w, h));
        fix_bottom_row(&mut img, w, h);
        assert!(!has_bottom_row_defect(&img, w, h));
        assert_eq!(row_luma(&img, w, h - 1), 255.0);
    }

    #[test]
    fn soft_gradient_is_not_a_defect() {
        let (w, h) = (4, 4);
        let mut img = solid(w, h, 200);
        let last = ((h - 1) * w * 4) as usize;
        img[last..].chunks_exact_mut(4).for_each(|p| p[..3].fill(180));
        assert!(!has_bottom_row_defect(&img, w, h));
    }

    #[test]
    fn still_image_fails_animated_check() {
        let facts = Facts { width: 100, height: 74, frame_count: 1, ..Default::default() };
        let checks = run(&facts, &Limits::default());
        assert_eq!(checks[0].status, Status::Fail);
    }

    #[test]
    fn poster_ratio_mismatch_fails() {
        let facts = Facts { width: 1200, height: 888, frame_count: 10, poster_size: Some((1200, 960)), ..Default::default() };
        let check = run(&facts, &Limits::default()).into_iter().find(|c| c.id == CheckId::PosterRatio).unwrap();
        assert_eq!(check.status, Status::Fail);
    }
}
