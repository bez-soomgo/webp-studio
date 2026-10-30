//! 영상(mp4·mov) 입력에만 쓰는 ffmpeg 호출.
//!
//! 앱에는 정적 빌드 ffmpeg 를 sidecar 로 묶는다(`scripts/fetch-ffmpeg.sh`). 번들 안에서는 실행 파일 옆
//! (`Contents/MacOS/ffmpeg`)에 놓이고, `tauri dev` 에서도 `target/debug/` 에 복사된다.
//! 애니메이션 WebP 는 ffmpeg 가 읽지 못하므로 여기를 거치지 않는다.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use anyhow::{bail, Context, Result};

static FFMPEG_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

/// 번들 sidecar → Homebrew 순으로 ffmpeg 를 찾는다. GUI 앱은 셸 PATH 를 물려받지 않아 경로를 직접 본다.
pub fn locate() -> Option<&'static Path> {
    FFMPEG_PATH
        .get_or_init(|| {
            let beside_exe = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join("ffmpeg")));
            let candidates = beside_exe
                .into_iter()
                .chain(["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"].map(PathBuf::from));
            candidates.into_iter().find(|p| p.is_file())
        })
        .as_deref()
}

fn ffmpeg() -> Result<Command> {
    match locate() {
        Some(path) => Ok(Command::new(path)),
        None => bail!("영상을 읽을 ffmpeg 를 찾지 못했습니다. 앱을 다시 설치하거나 WebP·PNG 폴더로 넣어 주세요."),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_ms: i32,
}

/// `ffmpeg -i` 가 stderr 로 내는 스트림 정보에서 해상도·fps·길이를 읽는다(ffprobe 를 따로 묶지 않으려고).
pub fn probe(path: &Path) -> Result<VideoInfo> {
    let output = ffmpeg()?.arg("-hide_banner").arg("-i").arg(path).output().context("ffmpeg 실행 실패")?;
    let text = String::from_utf8_lossy(&output.stderr);
    parse_probe(&text).with_context(|| format!("영상 정보를 읽지 못했습니다: {}", path.display()))
}

pub(crate) fn parse_probe(text: &str) -> Result<VideoInfo> {
    let video_line = text.lines().find(|l| l.contains("Video:")).context("영상 스트림이 없습니다")?;

    let (width, height) = video_line
        .split([',', ' '])
        .find_map(|tok| {
            let (w, h) = tok.split_once('x')?;
            Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?))
        })
        .context("해상도를 찾지 못했습니다")?;

    let fps = video_line
        .split(',')
        .find_map(|part| part.trim().strip_suffix(" fps").and_then(|v| v.trim().parse::<f64>().ok()))
        .or_else(|| video_line.split(',').find_map(|part| part.trim().strip_suffix(" tbr").and_then(|v| v.trim().parse::<f64>().ok())))
        .context("fps 를 찾지 못했습니다")?;

    let duration_ms = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("Duration: "))
        .and_then(|rest| rest.split(',').next())
        .and_then(parse_timecode)
        .unwrap_or(0);

    Ok(VideoInfo { width, height, fps, duration_ms })
}

fn parse_timecode(tc: &str) -> Option<i32> {
    let mut parts = tc.trim().split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let s: f64 = parts.next()?.parse().ok()?;
    Some(((h * 3600.0 + m * 60.0 + s) * 1000.0).round() as i32)
}

/// 영상을 `dir/00001.png …` 로 뽑는다. `fps` 가 있으면 그 fps 로 솎아 낸다.
pub fn extract_frames(path: &Path, dir: &Path, fps: Option<f64>) -> Result<()> {
    let mut cmd = ffmpeg()?;
    cmd.args(["-hide_banner", "-loglevel", "error", "-i"]).arg(path);
    if let Some(fps) = fps {
        cmd.arg("-vf").arg(format!("fps={fps}"));
    }
    cmd.args(["-fps_mode", "passthrough", "-an"]).arg(dir.join("%05d.png"));
    let output = cmd.output().context("ffmpeg 실행 실패")?;
    if !output.status.success() {
        bail!("영상에서 프레임을 뽑지 못했습니다: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ffmpeg_stream_info() {
        let text = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'a.mp4':\n  Duration: 00:00:05.03, start: 0.000000, bitrate: 1200 kb/s\n  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(tv, bt709, progressive), 1200x888 [SAR 1:1 DAR 50:37], 1150 kb/s, 30 fps, 30 tbr, 15360 tbn (default)\n";
        let info = parse_probe(text).unwrap();
        assert_eq!(info, VideoInfo { width: 1200, height: 888, fps: 30.0, duration_ms: 5030 });
    }
}
