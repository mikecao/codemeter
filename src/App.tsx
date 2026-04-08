import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type ServiceResult =
  | { status: "ok"; five_hour: number; weekly: number }
  | { status: "not_logged_in"; login_hint: string }
  | { status: "error"; message: string };

interface AllUsage {
  claude: ServiceResult;
  codex: ServiceResult;
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
          </div>
          <div className="metric">
            <div className="metric-label">
              <span>Weekly limit</span>
              <span>{Math.round(result.weekly)}% used</span>
            </div>
            <Bar percent={result.weekly} />
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
