import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";

import {
  api,
  CHECK_LABEL,
  fileUrl,
  formatMB,
  KIND_LABEL,
  type Check,
  type ConvertOptions,
  type ConvertResponse,
  type Inspection,
  type Limits,
} from "./api";
import "./App.css";

type Phase = "inspecting" | "ready" | "converting" | "done" | "error";

interface Item {
  id: string;
  path: string;
  name: string;
  phase: Phase;
  before?: Inspection;
  result?: ConvertResponse;
  progress: number;
  error?: string;
}

interface Preset {
  id: string;
  label: string;
  options: Pick<ConvertOptions, "width" | "fps" | "quality">;
  displayWidth: number;
}

/** 1200×888·30fps 3.30MB 파일로 확인한 값: 760px·20fps·품질 80 → 1.23MB. */
const PRESETS: Preset[] = [
  { id: "mweb", label: "모바일 웹 카드 (표시 폭 380px)", options: { width: 760, fps: 20, quality: 80 }, displayWidth: 380 },
  { id: "sharp", label: "3배율 폰까지 선명하게", options: { width: 1000, fps: 24, quality: 80 }, displayWidth: 380 },
  { id: "keep", label: "크기·fps 유지, 압축만", options: { width: null, fps: null, quality: 80 }, displayWidth: 380 },
];

const ACCEPTED = ["webp", "mp4", "mov", "m4v", "webm", "gif"];
let seq = 0;
const newId = () => `${Date.now().toString(36)}-${seq++}`;
const baseName = (path: string) => path.split(/[\\/]/).pop() ?? path;

function failCount(checks?: Check[]) {
  return checks?.filter((c) => c.status === "fail").length ?? 0;
}

function CheckChips({ checks }: { checks: Check[] }) {
  return (
    <ul className="chips">
      {checks.map((c) => (
        <li key={c.id} className={`chip ${c.status}`} title={c.detail}>
          <span className="chip-mark" aria-hidden="true">
            {c.status === "pass" ? "✓" : c.status === "fail" ? "!" : "–"}
          </span>
          {CHECK_LABEL[c.id]}
        </li>
      ))}
    </ul>
  );
}

function FactLine({ inspection }: { inspection: Inspection }) {
  const { facts, fps } = inspection;
  return (
    <dl className="facts">
      <div>
        <dt>해상도</dt>
        <dd>
          {facts.width}×{facts.height}
        </dd>
      </div>
      <div>
        <dt>프레임</dt>
        <dd>{facts.frameCount}</dd>
      </div>
      <div>
        <dt>fps</dt>
        <dd>{fps ? fps.toFixed(1) : "–"}</dd>
      </div>
      <div>
        <dt>용량</dt>
        <dd>{formatMB(facts.bytes)}</dd>
      </div>
    </dl>
  );
}

function Preview({ item, bust }: { item: Item; bust: number }) {
  const kind = item.before?.kind;
  return (
    <div className="previews">
      <figure>
        <figcaption>원본</figcaption>
        <div className="stage">
          {kind === "webp" && <img src={fileUrl(item.path, bust)} alt="" />}
          {kind === "video" && <video key={bust} src={fileUrl(item.path)} autoPlay loop muted playsInline />}
          {kind === "pngSequence" && <span className="stage-empty">PNG 폴더</span>}
          {!kind && <span className="stage-empty">읽는 중</span>}
        </div>
      </figure>
      <figure>
        <figcaption>변환본</figcaption>
        <div className="stage">
          {item.result ? (
            <img src={fileUrl(item.result.webpPath, bust)} alt="" />
          ) : (
            <span className="stage-empty">
              {item.phase === "converting" ? `${item.progress}프레임 처리 중` : "아직 변환 전"}
            </span>
          )}
        </div>
      </figure>
    </div>
  );
}

function ItemCard({ item, bust, onRemove }: { item: Item; bust: number; onRemove: () => void }) {
  const after = item.result?.outcome.output;
  const saved = item.before && after ? 1 - after.facts.bytes / item.before.facts.bytes : null;
  return (
    <article className={`card ${item.phase}`}>
      <header className="card-head">
        <div className="card-title">
          {item.before && <span className="kind">{KIND_LABEL[item.before.kind]}</span>}
          <h3 title={item.path}>{item.name}</h3>
        </div>
        <div className="card-actions">
          {saved !== null && (
            <span className={`saving ${saved > 0 ? "down" : "up"}`}>
              {formatMB(item.before!.facts.bytes)} → {formatMB(after!.facts.bytes)}
              <b>{saved > 0 ? `−${Math.round(saved * 100)}%` : `+${Math.round(-saved * 100)}%`}</b>
            </span>
          )}
          <button className="ghost small" onClick={onRemove} aria-label={`${item.name} 목록에서 빼기`}>
            빼기
          </button>
        </div>
      </header>

      {item.error && <p className="error">{item.error}</p>}

      <Preview item={item} bust={bust} />

      <div className="compare">
        <section>
          <h4>받은 파일</h4>
          {item.before ? (
            <>
              <FactLine inspection={item.before} />
              <CheckChips checks={item.before.checks} />
            </>
          ) : (
            <p className="muted">검사 중…</p>
          )}
        </section>
        <section>
          <h4>
            변환 결과
            {item.result && item.result.outcome.fixedBottomRows > 0 && (
              <span className="note"> · 아래쪽 줄 {item.result.outcome.fixedBottomRows}프레임 보정</span>
            )}
          </h4>
          {after ? (
            <>
              <FactLine inspection={after} />
              <CheckChips checks={after.checks} />
            </>
          ) : (
            <p className="muted">{item.phase === "converting" ? "변환 중…" : "변환하면 여기서 다시 검사합니다."}</p>
          )}
        </section>
      </div>
    </article>
  );
}

export default function App() {
  const [items, setItems] = useState<Item[]>([]);
  const [presetId, setPresetId] = useState(PRESETS[0].id);
  const [options, setOptions] = useState<ConvertOptions>({ ...PRESETS[0].options, fixBottomRow: true, sequenceFps: 30 });
  const [limits, setLimits] = useState<Limits>({ maxBytes: 1_500_000, displayWidth: 380 });
  const [busy, setBusy] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [bust, setBust] = useState(0);
  const [toast, setToast] = useState<string | null>(null);
  const [hasFfmpeg, setHasFfmpeg] = useState(true);
  const initialLoaded = useRef(false);
  const limitsRef = useRef(limits);
  limitsRef.current = limits;
  const optionsRef = useRef(options);
  optionsRef.current = options;

  const patch = useCallback((id: string, next: Partial<Item>) => {
    setItems((prev) => prev.map((it) => (it.id === id ? { ...it, ...next } : it)));
  }, []);

  const addPaths = useCallback(
    (paths: string[]) => {
      const fresh: Item[] = paths.map((path) => ({ id: newId(), path, name: baseName(path), phase: "inspecting", progress: 0 }));
      setItems((prev) => [...prev, ...fresh]);
      for (const it of fresh) {
        api
          .inspect(it.path, limitsRef.current, optionsRef.current.sequenceFps)
          .then((before) => patch(it.id, { before, phase: "ready" }))
          .catch((e) => patch(it.id, { phase: "error", error: String(e) }));
      }
    },
    [patch],
  );

  useEffect(() => {
    api.ffmpegAvailable().then(setHasFfmpeg).catch(() => setHasFfmpeg(false));
    const unProgress = listen<{ id: string; frames: number }>("convert-progress", (e) =>
      patch(e.payload.id, { progress: e.payload.frames }),
    );
    const unDrop = getCurrentWebview().onDragDropEvent((e) => {
      if (e.payload.type === "over" || e.payload.type === "enter") setDragging(true);
      else if (e.payload.type === "leave") setDragging(false);
      else if (e.payload.type === "drop") {
        setDragging(false);
        addPaths(e.payload.paths);
      }
    });
    return () => {
      unProgress.then((f) => f());
      unDrop.then((f) => f());
    };
  }, [addPaths, patch]);

  useEffect(() => {
    // StrictMode 에서 effect 가 두 번 돌아도 한 번만 넣도록 막는다.
    if (initialLoaded.current) return;
    initialLoaded.current = true;
    api.initialPaths().then((paths) => paths.length && addPaths(paths));
  }, [addPaths]);

  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 3200);
    return () => clearTimeout(t);
  }, [toast]);

  const pickFiles = async () => {
    const picked = await open({ multiple: true, filters: [{ name: "WebP·영상", extensions: ACCEPTED }] });
    if (picked) addPaths(Array.isArray(picked) ? picked : [picked]);
  };

  const pickFolder = async () => {
    const picked = await open({ directory: true, multiple: true });
    if (picked) addPaths(Array.isArray(picked) ? picked : [picked]);
  };

  const applyPreset = (id: string) => {
    const preset = PRESETS.find((p) => p.id === id);
    setPresetId(id);
    if (!preset) return;
    setOptions((o) => ({ ...o, ...preset.options }));
    setLimits((l) => ({ ...l, displayWidth: preset.displayWidth }));
  };

  const editOption = <K extends keyof ConvertOptions>(key: K, value: ConvertOptions[K]) => {
    setPresetId("custom");
    setOptions((o) => ({ ...o, [key]: value }));
  };

  const convertAll = async () => {
    setBusy(true);
    // 한 번에 하나씩 — 큰 파일 여러 개를 동시에 풀면 메모리를 많이 쓴다.
    for (const it of items) {
      if (it.phase === "inspecting" || it.phase === "converting") continue;
      if (it.phase === "error" && !it.before) continue;
      patch(it.id, { phase: "converting", progress: 0, error: undefined, result: undefined });
      try {
        const result = await api.convert(it.id, it.path, options, limits);
        patch(it.id, { phase: "done", result });
      } catch (e) {
        patch(it.id, { phase: "error", error: String(e) });
      }
    }
    setBust((b) => b + 1);
    setBusy(false);
  };

  const done = items.filter((it) => it.result);

  const saveAll = async () => {
    const dest = await open({ directory: true, title: "저장할 폴더 고르기" });
    if (!dest || Array.isArray(dest)) return;
    const report = {
      createdAt: new Date().toISOString(),
      options,
      limits,
      files: done.map((it) => ({
        input: it.name,
        before: it.before && { facts: it.before.facts, checks: it.before.checks },
        after: { facts: it.result!.outcome.output.facts, checks: it.result!.outcome.output.checks },
        fixedBottomRows: it.result!.outcome.fixedBottomRows,
      })),
    };
    try {
      const count = await api.save(
        done.map((it) => ({ webpPath: it.result!.webpPath, posterPath: it.result!.posterPath })),
        dest,
        report,
      );
      setToast(`${count}개 파일과 report.json 을 저장했습니다.`);
    } catch (e) {
      setToast(`저장하지 못했습니다: ${e}`);
    }
  };

  const summary = useMemo(() => {
    const failing = done.filter((it) => failCount(it.result!.outcome.output.checks) > 0).length;
    const before = done.reduce((s, it) => s + (it.before?.facts.bytes ?? 0), 0);
    const after = done.reduce((s, it) => s + it.result!.outcome.output.facts.bytes, 0);
    return { failing, before, after };
  }, [done]);

  const convertible = items.some((it) => it.phase === "ready" || it.phase === "done" || (it.phase === "error" && it.before));

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <h1>WebP Studio</h1>
          <p>애니메이션 WebP 를 줄이고, 넘기기 전에 검사합니다.</p>
        </div>
        <div className="top-actions">
          <button className="ghost" onClick={pickFiles}>
            파일 추가
          </button>
          <button className="ghost" onClick={pickFolder}>
            PNG 폴더 추가
          </button>
          <button className="primary" onClick={convertAll} disabled={busy || !convertible}>
            {busy ? "변환 중…" : "모두 변환"}
          </button>
          <button className="primary alt" onClick={saveAll} disabled={busy || done.length === 0}>
            저장…
          </button>
        </div>
      </header>

      <div className="layout">
        <aside className="settings" aria-label="변환 설정">
          <h2>변환 설정</h2>
          <label className="field">
            <span>프리셋</span>
            <select id="preset" value={presetId} onChange={(e) => applyPreset(e.target.value)}>
              {PRESETS.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.label}
                </option>
              ))}
              <option value="custom">직접 설정</option>
            </select>
          </label>

          <label className="field">
            <span>출력 폭</span>
            <div className="with-unit">
              <input
                id="width"
                type="number"
                min={64}
                step={2}
                placeholder="원본"
                value={options.width ?? ""}
                onChange={(e) => editOption("width", e.target.value ? Number(e.target.value) : null)}
              />
              <em>px</em>
            </div>
            <small>원본보다 크면 키우지 않습니다.</small>
          </label>

          <label className="field">
            <span>fps</span>
            <select
              id="fps"
              value={options.fps ?? ""}
              onChange={(e) => editOption("fps", e.target.value ? Number(e.target.value) : null)}
            >
              <option value="">원본 유지</option>
              <option value="30">30</option>
              <option value="24">24</option>
              <option value="20">20</option>
              <option value="15">15</option>
            </select>
          </label>

          <label className="field">
            <span>
              품질 <output>{options.quality}</output>
            </span>
            <input
              id="quality"
              type="range"
              min={40}
              max={100}
              step={5}
              value={options.quality}
              onChange={(e) => editOption("quality", Number(e.target.value))}
            />
          </label>

          <label className="toggle">
            <input
              id="fix-bottom"
              type="checkbox"
              checked={options.fixBottomRow}
              onChange={(e) => editOption("fixBottomRow", e.target.checked)}
            />
            <span>아래쪽 1px 줄 자동 보정</span>
          </label>

          <h2>검사 기준</h2>
          <label className="field">
            <span>용량 상한</span>
            <div className="with-unit">
              <input
                id="max-mb"
                type="number"
                min={0.1}
                step={0.1}
                value={limits.maxBytes / 1_000_000}
                onChange={(e) => setLimits((l) => ({ ...l, maxBytes: Math.round(Number(e.target.value) * 1_000_000) }))}
              />
              <em>MB</em>
            </div>
          </label>
          <label className="field">
            <span>화면 표시 폭</span>
            <div className="with-unit">
              <input
                id="display-width"
                type="number"
                min={100}
                value={limits.displayWidth}
                onChange={(e) => setLimits((l) => ({ ...l, displayWidth: Number(e.target.value) }))}
              />
              <em>px</em>
            </div>
            <small>해상도 검사는 이 폭의 3배까지 통과입니다.</small>
          </label>
          <label className="field">
            <span>PNG 폴더 fps</span>
            <div className="with-unit">
              <input
                id="sequence-fps"
                type="number"
                min={1}
                max={60}
                value={options.sequenceFps}
                onChange={(e) => editOption("sequenceFps", Number(e.target.value))}
              />
              <em>fps</em>
            </div>
          </label>

          {!hasFfmpeg && <p className="warn">ffmpeg 가 없어 mp4·mov 는 읽을 수 없습니다. WebP 와 PNG 폴더는 쓸 수 있습니다.</p>}
        </aside>

        <main className="list">
          {done.length > 0 && (
            <div className="summary">
              <span>
                {done.length}개 변환 · {formatMB(summary.before)} → <b>{formatMB(summary.after)}</b>
              </span>
              {summary.failing > 0 ? (
                <span className="summary-bad">검사 실패 {summary.failing}개 — 칩에 마우스를 올리면 이유가 보입니다</span>
              ) : (
                <span className="summary-good">모두 검사 통과</span>
              )}
              <button className="ghost small" onClick={() => setBust((b) => b + 1)}>
                처음부터 다시 재생
              </button>
            </div>
          )}

          {items.length === 0 ? (
            <button className={`dropzone ${dragging ? "over" : ""}`} onClick={pickFiles}>
              <strong>WebP·영상 파일이나 PNG 폴더를 여기로 끌어다 놓으세요</strong>
              <span>WebP · mp4 · mov · PNG 시퀀스 폴더</span>
            </button>
          ) : (
            <>
              {dragging && <div className="drop-overlay">놓으면 목록에 추가합니다</div>}
              {items.map((it) => (
                <ItemCard
                  key={it.id}
                  item={it}
                  bust={bust}
                  onRemove={() => setItems((prev) => prev.filter((p) => p.id !== it.id))}
                />
              ))}
            </>
          )}
        </main>
      </div>

      {toast && (
        <div className="toast" role="status">
          {toast}
        </div>
      )}
    </div>
  );
}
