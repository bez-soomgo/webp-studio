//! 화면에서 부르는 Tauri command. 무거운 처리는 blocking 스레드로 넘긴다.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::checks::Limits;
use crate::process::{self, ConvertOptions, ConvertOutcome, Inspection};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    // anyhow 의 context 사슬까지 보여 주려고 `{:#}` 를 쓴다.
    format!("{e:#}")
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(err)?.map_err(err)
}

/// 실행 인자로 받은 파일·폴더(`WebP Studio a.webp b.mov`). 앱을 열자마자 목록에 올린다.
#[tauri::command]
pub fn initial_paths() -> Vec<String> {
    std::env::args().skip(1).filter(|a| !a.starts_with('-') && Path::new(a).exists()).collect()
}

#[tauri::command]
pub fn ffmpeg_available() -> bool {
    crate::ffmpeg::locate().is_some()
}

#[tauri::command]
pub async fn inspect_file(path: String, poster: Option<String>, limits: Limits, sequence_fps: f64) -> CmdResult<Inspection> {
    blocking(move || process::inspect(Path::new(&path), poster.as_deref().map(Path::new), &limits, sequence_fps)).await
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    id: String,
    frames: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertResponse {
    pub webp_path: String,
    pub poster_path: String,
    pub outcome: ConvertOutcome,
}

fn stem(path: &Path) -> String {
    path.file_stem().and_then(|s| s.to_str()).unwrap_or("output").to_string()
}

/// 결과는 앱 캐시 폴더(`jobs/<id>/`)에 먼저 쓰고, 사용자가 저장을 누르면 그때 복사한다.
#[tauri::command]
pub async fn convert_file(
    app: AppHandle,
    id: String,
    path: String,
    options: ConvertOptions,
    limits: Limits,
) -> CmdResult<ConvertResponse> {
    let job_dir = app.path().app_cache_dir().map_err(err)?.join("jobs").join(&id);
    std::fs::create_dir_all(&job_dir).map_err(err)?;
    let input = PathBuf::from(&path);
    let name = stem(&input);
    let webp = job_dir.join(format!("{name}.webp"));
    let poster = job_dir.join(format!("{name}.png"));

    let (webp_c, poster_c) = (webp.clone(), poster.clone());
    let outcome = blocking(move || {
        process::convert(&input, &options, &limits, &webp_c, &poster_c, |frames| {
            if frames % 5 == 0 {
                let _ = app.emit("convert-progress", Progress { id: id.clone(), frames });
            }
        })
    })
    .await?;

    Ok(ConvertResponse {
        webp_path: webp.to_string_lossy().into(),
        poster_path: poster.to_string_lossy().into(),
        outcome,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveItem {
    webp_path: String,
    poster_path: String,
}

/// 변환 결과를 고른 폴더로 복사하고 검사 결과를 `report.json` 으로 남긴다. 저장한 파일 수를 돌려준다.
#[tauri::command]
pub async fn save_outputs(items: Vec<SaveItem>, dest_dir: String, report: serde_json::Value) -> CmdResult<usize> {
    blocking(move || {
        let dest = PathBuf::from(dest_dir);
        let mut count = 0;
        for item in &items {
            for src in [&item.webp_path, &item.poster_path] {
                let src = Path::new(src);
                let name = src.file_name().ok_or_else(|| anyhow::anyhow!("파일 이름이 없습니다"))?;
                std::fs::copy(src, dest.join(name))?;
                count += 1;
            }
        }
        std::fs::write(dest.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
        Ok(count)
    })
    .await
}
