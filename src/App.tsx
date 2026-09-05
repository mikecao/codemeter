import React, { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { ClaudeIcon, GrokIcon, OpenAIIcon, OpenCodeIcon } from "./icons";

// Must match .info-panel / grid values in style.css
const CARD_MIN_WIDTH = 380;
const GRID_GAP = 14;
const PANEL_PADDING = 16;
const MAX_COLS = 3;
// Small slack so DPI rounding can't push the grid below the column threshold.
const WIDTH_SLACK = 4;

/** Window inner width needed to show `cols` columns at CARD_MIN_WIDTH each. */
function widthForCols(cols: number): number {
  return cols * CARD_MIN_WIDTH + (cols - 1) * GRID_GAP + PANEL_PADDING * 2 + WIDTH_SLACK;
}

/** How many columns fit in a given window inner width (used on manual resize). */
function colsForWidth(width: number): number {
  const fit = Math.floor((width - PANEL_PADDING * 2 + GRID_GAP) / (CARD_MIN_WIDTH + GRID_GAP));
  return Math.max(1, Math.min(MAX_COLS, fit));
}

/** Preferred column count for a given number of cards (roughly square). */
function colsForCards(count: number): number {
  return Math.min(MAX_COLS, Math.ceil(Math.sqrt(Math.max(1, count))));
}

interface UsageWindow {
  label: string;
  percent: number;
  resets_at: string | null;
}

type ServiceResult =
  | { status: "ok"; windows: UsageWindow[] }
  | { status: "not_logged_in"; login_hint: string }
  | { status: "not_installed" }
  | { status: "error"; message: string };

interface AllUsage {
  claude: ServiceResult;
  codex: ServiceResult;
  opencode: ServiceResult;
  grok: ServiceResult;
}

function formatCountdown(iso: string): string {
  const diffMs = new Date(iso).getTime() - Date.now();
  if (diffMs <= 0) return "now";
  const mins = Math.floor(diffMs / 60_000);
  const hrs = Math.floor(mins / 60);
  const remainMins = mins % 60;
  const days = Math.floor(hrs / 24);
  const remainHrs = hrs % 24;
  if (days > 0) return `${days}d ${remainHrs}h`;
  if (hrs > 0) return `${hrs}h ${remainMins}m`;
  return `${mins}m`;
}

function formatDateTime(iso: string): string {
  const d = new Date(iso);
  return d.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

function Bar({ percent }: { percent: number }) {
  const clamped = Math.min(100, Math.max(0, percent));
  const color = clamped > 80 ? "#ef4444" : clamped > 50 ? "#eab308" : "#22c55e";

  return (
    <div className="bar">
      <div
        className="bar-fill"
        style={{ width: `${clamped}%`, background: color }}
      />
    </div>
  );
}

const HOUR_MS = 3_600_000;
const DAY_MS = 24 * HOUR_MS;

// One dot per unit of the window (7 days, 5 hours); filled = units remaining.
const WINDOW_DOTS: { prefix: string; count: number; unitMs: number; unit: string }[] = [
  { prefix: "Weekly", count: 7, unitMs: DAY_MS, unit: "day" },
  { prefix: "5h", count: 5, unitMs: HOUR_MS, unit: "hour" },
];

function WindowDots({
  resetsAt,
  count,
  unitMs,
  unit,
}: {
  resetsAt: string;
  count: number;
  unitMs: number;
  unit: string;
}) {
  const diffMs = new Date(resetsAt).getTime() - Date.now();
  const remaining = Math.min(count, Math.max(0, diffMs / unitMs));
  return (
    <span className="day-dots" title={`${remaining.toFixed(1)} of ${count} ${unit}s left`}>
      {Array.from({ length: count }, (_, i) => {
        // Dot i covers units [i, i+1); fill it by however much of that unit remains.
        const fill = Math.min(1, Math.max(0, remaining - i));
        return (
          <span
            key={i}
            className="day-dot"
            style={{ "--fill": `${Math.round(fill * 100)}%` } as React.CSSProperties}
          />
        );
      })}
    </span>
  );
}

function Metric({ window }: { window: UsageWindow }) {
  const dots = WINDOW_DOTS.find((d) => window.label.startsWith(d.prefix));
  return (
    <div className="metric">
      <div className="metric-label">
        <span>{window.label}</span>
        <span>{Math.round(window.percent)}% used</span>
      </div>
      <Bar percent={window.percent} />
      {window.resets_at && (
        <div className="resets">
          <span className="resets-left">
            <span>Resets in {formatCountdown(window.resets_at)}</span>
            {dots && <WindowDots resetsAt={window.resets_at} {...dots} />}
          </span>
          <span>{formatDateTime(window.resets_at)}</span>
        </div>
      )}
    </div>
  );
}

function Service({
  name,
  icon,
  result,
}: {
  name: string;
  icon: ReactNode;
  result: ServiceResult;
}) {
  return (
    <div className="service">
      <div className="service-header">
        {icon}
        <span>{name}</span>
      </div>
      {result.status === "ok" ? (
        result.windows.map((w) => <Metric key={w.label} window={w} />)
      ) : result.status === "not_logged_in" ? (
        <div className="hint">{result.login_hint}</div>
      ) : result.status === "error" ? (
        <div className="hint">{result.message}</div>
      ) : null}
    </div>
  );
}

const NO_PROVIDERS_HINT =
  "No supported CLI found. Install and log in to Claude Code, Codex CLI, OpenCode or Grok.";

function visibleServices(usage: AllUsage) {
  return [
    { name: "Claude Code", icon: <ClaudeIcon />, result: usage.claude },
    { name: "Codex CLI", icon: <OpenAIIcon />, result: usage.codex },
    { name: "OpenCode Go", icon: <OpenCodeIcon />, result: usage.opencode },
    { name: "Grok", icon: <GrokIcon />, result: usage.grok },
  ].filter((s) => s.result.status !== "not_installed");
}

export function App() {
  const [usage, setUsage] = useState<AllUsage | null>(null);
  const [cols, setCols] = useState(() => colsForWidth(globalThis.innerWidth));
  const panelRef = useRef<HTMLDivElement>(null);
  const sizedFor = useRef<string>("");

  // Manual window resizes re-derive the column count from the real width.
  useEffect(() => {
    const onResize = () => setCols(colsForWidth(globalThis.innerWidth));
    globalThis.addEventListener("resize", onResize);
    return () => globalThis.removeEventListener("resize", onResize);
  }, []);

  // Size the window to fit the cards whenever their shape changes
  // (first load, or a service logging in/out). Manual resizes are left alone otherwise.
  useLayoutEffect(() => {
    const panel = panelRef.current;
    if (!usage || !panel) return;

    const results = visibleServices(usage).map((s) => s.result);
    const signature = results
      .map((r) => (r.status === "ok" ? r.windows.length : r.status))
      .join(",");
    if (signature === sizedFor.current) return;
    sizedFor.current = signature;

    const targetCols = colsForCards(results.length);
    const width = widthForCols(targetCols);

    // Lay the panel out exactly as it will appear after the resize (explicit
    // column count + target width) and measure the resulting height. Nothing
    // here depends on the current viewport, so scrollbars/DPI can't skew it.
    // (.info-panel is border-box; macOS pins min-height to the viewport.)
    panel.style.setProperty("--cols", String(targetCols));
    panel.style.width = `${width}px`;
    panel.style.minHeight = "0";
    const height = Math.ceil(panel.getBoundingClientRect().height);
    panel.style.width = "";
    panel.style.minHeight = "";

    setCols(targetCols);
    getCurrentWindow()
      .setSize(new LogicalSize(width, height))
      .catch((e) => console.error("setSize failed", e));
  }, [usage]);

  useEffect(() => {
    const fetch = () => {
      invoke<AllUsage>("get_usage").then(setUsage).catch(() => {});
    };
    fetch();
    const id = setInterval(fetch, 60_000);
    return () => clearInterval(id);
  }, []);

  if (!usage) {
    return <div className="info-panel"><div className="hint">Loading...</div></div>;
  }

  const services = visibleServices(usage);

  return (
    <div
      className="info-panel"
      ref={panelRef}
      style={{ "--cols": String(cols) } as React.CSSProperties}
    >
      {services.length === 0 ? (
        <div className="hint">{NO_PROVIDERS_HINT}</div>
      ) : (
        services.map((s) => (
          <Service key={s.name} name={s.name} icon={s.icon} result={s.result} />
        ))
      )}
    </div>
  );
}
