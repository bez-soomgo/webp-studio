import { convertFileSrc, invoke } from "@tauri-apps/api/core";

/** Rust `checks.rs` 와 같은 모양. */
export type CheckId = "animated" | "bottomRow" | "fileSize" | "posterRatio" | "resolution";
export type CheckStatus = "pass" | "fail" | "skip";
export type SourceKind = "webp" | "video" | "pngSequence";

export interface Check {
  id: CheckId;
  status: CheckStatus;
  detail: string;
}

export interface Facts {
  width: number;
  height: number;
  frameCount: number;
  durationMs: number;
  bytes: number;
  bottomRowDefectFrames: number;
  posterSize: [number, number] | null;
}

export interface Inspection {
  kind: SourceKind;
  facts: Facts;
  fps: number;
  checks: Check[];
}

export interface Limits {
  maxBytes: number;
  displayWidth: number;
}

export interface ConvertOptions {
  width: number | null;
  fps: number | null;
  quality: number;
  fixBottomRow: boolean;
  sequenceFps: number;
}

export interface ConvertResponse {
  webpPath: string;
  posterPath: string;
  outcome: {
    framesIn: number;
    framesOut: number;
    fixedBottomRows: number;
    output: Inspection;
  };
}

export const api = {
  ffmpegAvailable: () => invoke<boolean>("ffmpeg_available"),
  initialPaths: () => invoke<string[]>("initial_paths"),
  inspect: (path: string, limits: Limits, sequenceFps: number) =>
    invoke<Inspection>("inspect_file", { path, poster: null, limits, sequenceFps }),
  convert: (id: string, path: string, options: ConvertOptions, limits: Limits) =>
    invoke<ConvertResponse>("convert_file", { id, path, options, limits }),
  save: (items: { webpPath: string; posterPath: string }[], destDir: string, report: unknown) =>
    invoke<number>("save_outputs", { items, destDir, report }),
};

/** 로컬 파일을 웹뷰에서 그린다. `bust` 를 바꾸면 애니메이션이 처음부터 다시 재생된다. */
export const fileUrl = (path: string, bust = 0) => `${convertFileSrc(path)}?v=${bust}`;

export const CHECK_LABEL: Record<CheckId, string> = {
  animated: "애니메이션",
  bottomRow: "아래쪽 줄",
  fileSize: "용량",
  posterRatio: "정지본 비율",
  resolution: "해상도",
};

export const KIND_LABEL: Record<SourceKind, string> = {
  webp: "WebP",
  video: "영상",
  pngSequence: "PNG 폴더",
};

export const formatMB = (bytes: number) => `${(bytes / 1_000_000).toFixed(2)}MB`;
