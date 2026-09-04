import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { ClaudeIcon, GrokIcon, OpenAIIcon, OpenCodeIcon } from "./icons";

// Must match .info-panel / grid values in style.css
const CARD_MIN_WIDTH = 380;
const GRID_GAP = 14;
const PANEL_PADDING = 16;
const MAX_COLS = 3;

interface UsageWindow {
  label: string;
  percent: number;
  resets_at: string | null;
}

type ServiceResult =
  | { status: "ok"; windows: UsageWindow[] }
  | { status: "not_logged_in"; login_hint: string }
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

function Metric({ window }: { window: UsageWindow }) {
  return (
    <div className="metric">
      <div className="metric-label">
        <span>{window.label}</span>
        <span>{Math.round(window.percent)}% used</span>
      </div>
      <Bar percent={window.percent} />
      {window.resets_at && (
        <div className="resets">
          <span>Resets in {formatCountdown(window.resets_at)}</span>
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
      ) : (
        <div className="hint">{result.message}</div>
      )}
    </div>
  );
}

export function App() {
  const [usage, setUsage] = useState<AllUsage | null>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const sizedFor = useRef<string>("");

  // Size the window to fit the cards whenever their shape changes
  // (first load, or a service logging in/out). Manual resizes are left alone otherwise.
  useLayoutEffect(() => {
    const panel = panelRef.current;
    if (!usage || !panel) return;

    const results = [usage.claude, usage.codex, usage.opencode, usage.grok];
    const signature = results
      .map((r) => (r.status === "ok" ? r.windows.length : r.status))
      .join(",");
    if (signature === sizedFor.current) return;
    sizedFor.current = signature;

    const cols = Math.min(MAX_COLS, Math.ceil(Math.sqrt(results.length)));
    const width = cols * CARD_MIN_WIDTH + (cols - 1) * GRID_GAP + PANEL_PADDING * 2;

    // Lay the panel out at the target width to measure the resulting height.
    // (.info-panel is border-box, and macOS pins min-height to the viewport,
    // so override both while measuring.)
    panel.style.width = `${width}px`;
    panel.style.minHeight = "0";
    const height = Math.ceil(panel.getBoundingClientRect().height);
    panel.style.width = "";
    panel.style.minHeight = "";

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

  return (
    <div className="info-panel" ref={panelRef}>
      <Service name="Claude Code" icon={<ClaudeIcon />} result={usage.claude} />
      <Service name="Codex CLI" icon={<OpenAIIcon />} result={usage.codex} />
      <Service name="OpenCode Go" icon={<OpenCodeIcon />} result={usage.opencode} />
      <Service name="Grok" icon={<GrokIcon />} result={usage.grok} />
    </div>
  );
}
