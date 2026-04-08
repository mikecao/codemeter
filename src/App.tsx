import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type ServiceResult =
  | { status: "ok"; five_hour: number; five_hour_resets_at: string | null; weekly: number; weekly_resets_at: string | null }
  | { status: "not_logged_in"; login_hint: string }
  | { status: "error"; message: string };

interface AllUsage {
  claude: ServiceResult;
  codex: ServiceResult;
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

function Service({ name, result }: { name: string; result: ServiceResult }) {
  return (
    <div className="service">
      <div className="service-header">{name}</div>
      {result.status === "ok" ? (
        <>
          <div className="metric">
            <div className="metric-label">
              <span>5h limit</span>
              <span>{Math.round(result.five_hour)}% used</span>
            </div>
            <Bar percent={result.five_hour} />
            {result.five_hour_resets_at && (
              <div className="resets">
                <span>Resets in {formatCountdown(result.five_hour_resets_at)}</span>
                <span>{formatDateTime(result.five_hour_resets_at)}</span>
              </div>
            )}
          </div>
          <div className="metric">
            <div className="metric-label">
              <span>Weekly limit</span>
              <span>{Math.round(result.weekly)}% used</span>
            </div>
            <Bar percent={result.weekly} />
            {result.weekly_resets_at && (
              <div className="resets">
                <span>Resets in {formatCountdown(result.weekly_resets_at)}</span>
                <span>{formatDateTime(result.weekly_resets_at)}</span>
              </div>
            )}
          </div>
        </>
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
    <div className="info-panel">
      <Service name="Claude Code" result={usage.claude} />
      <Service name="Codex CLI" result={usage.codex} />
    </div>
  );
}
