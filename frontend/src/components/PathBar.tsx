import { useCallback, useEffect, useRef, useState } from "react";
import { Monitor, RefreshCw } from "lucide-react";

interface PathBarProps {
  hostname: string;
  cwd: string;
  connected?: boolean;
  onRefresh: () => void;
  onNavigateTo: (absolutePath: string) => void;
  fetchSuggestions?: (input: string) => Promise<string[]>;
}

export default function PathBar({ hostname, cwd, connected, onRefresh, onNavigateTo, fetchSuggestions }: PathBarProps) {
  const [inputValue, setInputValue] = useState(cwd);
  const [suggestions, setSuggestions] = useState<string[]>([]);
  const [activeIndex, setActiveIndex] = useState(-1);
  const [focused, setFocused] = useState(false);
  const debounceRef = useRef<ReturnType<typeof setTimeout>>(undefined);
  const inputRef = useRef<HTMLInputElement>(null);

  // Sync input with cwd when not focused (folder clicks, navigation)
  useEffect(() => {
    if (!focused) setInputValue(cwd);
  }, [cwd, focused]);

  const reset = useCallback(() => {
    setInputValue(cwd);
    setSuggestions([]);
    setActiveIndex(-1);
    clearTimeout(debounceRef.current);
  }, [cwd]);

  // Debounced autocomplete fetch (200ms) — only while focused
  useEffect(() => {
    if (!focused || !fetchSuggestions || !inputValue) {
      setSuggestions([]);
      return;
    }
    clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(async () => {
      const results = await fetchSuggestions(inputValue);
      setSuggestions(results);
      setActiveIndex(-1);
    }, 200);
    return () => clearTimeout(debounceRef.current);
  }, [inputValue, fetchSuggestions, focused]);

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      onNavigateTo(activeIndex >= 0 ? suggestions[activeIndex] : inputValue);
      inputRef.current?.blur();
    } else if (e.key === "Escape") {
      reset();
      inputRef.current?.blur();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setActiveIndex((i) => Math.min(i + 1, suggestions.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActiveIndex((i) => Math.max(i - 1, -1));
    } else if (e.key === "Tab" && suggestions.length > 0) {
      e.preventDefault();
      const selected = activeIndex >= 0 ? suggestions[activeIndex] : suggestions[0];
      setInputValue(selected + "/");
      setActiveIndex(-1);
    }
  };

  return (
    <div className="flex items-center gap-2 px-3 py-2 bg-zinc-800/50 border-b border-zinc-700/50 text-sm">
      <Monitor className="w-4 h-4 text-zinc-400 shrink-0" />
      <span className="font-semibold text-emerald-400">{hostname}</span>
      <span className="text-zinc-500">:</span>
      <div className="relative flex-1 min-w-0">
        <input
          ref={inputRef}
          value={inputValue}
          onChange={(e) => setInputValue(e.target.value)}
          onKeyDown={handleKeyDown}
          onFocus={(e) => { setFocused(true); e.target.select(); }}
          onBlur={() => { setFocused(false); reset(); }}
          className="w-full bg-transparent border border-transparent rounded px-2 py-0.5 font-mono text-xs text-zinc-300 focus:outline-none focus:border-zinc-600 focus:bg-zinc-900 focus:text-zinc-100 transition-colors"
        />
        {suggestions.length > 0 && focused && (
          <ul className="absolute top-full left-0 right-0 z-50 mt-0.5 max-h-48 overflow-y-auto bg-zinc-900 border border-zinc-700 rounded shadow-xl">
            {suggestions.map((s, i) => (
              <li
                key={s}
                className={`px-3 py-1.5 font-mono text-xs cursor-pointer truncate ${
                  i === activeIndex
                    ? "bg-emerald-500/20 text-emerald-300"
                    : "text-zinc-300 hover:bg-zinc-800"
                }`}
                onMouseDown={(e) => {
                  e.preventDefault(); // prevent input blur before click registers
                  onNavigateTo(s);
                }}
              >
                {s}
              </li>
            ))}
          </ul>
        )}
      </div>
      <button
        onClick={onRefresh}
        className="ml-auto p-1 hover:bg-zinc-700/50 rounded transition-colors"
        title="Refresh"
      >
        <RefreshCw className="w-3.5 h-3.5 text-zinc-400 hover:text-emerald-400" />
      </button>
      {connected !== undefined && (
        <span
          className={`w-2 h-2 rounded-full shrink-0 ${
            connected ? "bg-emerald-400" : "bg-red-400"
          }`}
        />
      )}
    </div>
  );
}
