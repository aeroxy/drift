import type { Transfer } from "../hooks/useTransfer";

interface TransferBarProps {
  transfers: Transfer[];
}

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / Math.pow(1024, i)).toFixed(1)} ${units[i]}`;
}

export default function TransferBar({ transfers }: TransferBarProps) {
  const active = transfers.filter((t) => t.status === "active");
  if (active.length === 0) return null;

  return (
    <div className="border-t border-zinc-700/50 bg-zinc-900/50">
      {active.map((t) => {
        const pct = t.bytes_total > 0 ? (t.bytes_done / t.bytes_total) * 100 : 0;
        return (
          <div key={t.id} className="px-3 py-2">
            <div className="flex items-center justify-between text-xs text-zinc-400 mb-1">
              <span className="truncate">{t.path}</span>
              <span className="tabular-nums ml-2">
                {formatBytes(t.bytes_done)} / {formatBytes(t.bytes_total)}
              </span>
            </div>
            <div className="h-1 bg-zinc-800 rounded-full overflow-hidden">
              <div
                className="h-full bg-emerald-400 rounded-full transition-all duration-300"
                style={{ width: `${pct}%` }}
              />
            </div>
          </div>
        );
      })}
    </div>
  );
}
