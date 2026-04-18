export interface FileEntry {
  name: string;
  is_dir: boolean;
  size: number;
  modified: number;
  permissions?: number;
}

export interface TransferEntry {
  relative_path: string;
  size: number;
  is_dir: boolean;
  permissions?: number;
}

export interface BrowseResponse {
  hostname: string;
  cwd: string;
  entries: FileEntry[];
}

export interface InfoResponse {
  hostname: string;
  root_dir: string;
  has_remote: boolean;
  fingerprint: string | null;
}

export interface ConnectRequest {
  target: string;
  password?: string;
}

export interface ConnectResponse {
  success: boolean;
  error?: string;
  fingerprint?: string;
}

export interface TransferProgress {
  id: string;
  path: string;
  bytes_done: number;
  bytes_total: number;
}

export type ControlMessage =
  | { type: "BrowseRequest"; path: string }
  | { type: "BrowseResponse"; hostname: string; cwd: string; entries: FileEntry[] }
  | { type: "InfoRequest" }
  | { type: "InfoResponse"; hostname: string; root_dir: string }
  | { type: "TransferRequest"; id: string; entries: TransferEntry[]; direction: "Push" | "Pull"; destination_path: string }
  | { type: "TransferProgress"; id: string; path: string; bytes_done: number; bytes_total: number }
  | { type: "TransferComplete"; id: string; total_bytes: number }
  | { type: "TransferFinalized"; id: string }
  | { type: "TransferError"; id: string; error: string }
  | { type: "ConnectionStatus"; has_remote: boolean }
  | { type: "Ping" }
  | { type: "Pong" }
  | { type: "Error"; message: string };
