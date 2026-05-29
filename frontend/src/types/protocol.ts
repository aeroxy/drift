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
  can_reconnect: boolean;
  last_target: string | null;
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
  | { type: "BrowseRequest"; request_id?: string | null; path: string }
  | { type: "BrowseResponse"; request_id?: string | null; hostname: string; cwd: string; entries: FileEntry[] }
  | { type: "InfoRequest"; request_id?: string | null }
  | { type: "InfoResponse"; request_id?: string | null; hostname: string; root_dir: string }
  | { type: "TransferRequest"; id: string; entries: TransferEntry[]; direction: "Push" | "Pull"; destination_path: string }
  | { type: "TransferProgress"; id: string; path: string; bytes_done: number; bytes_total: number }
  | { type: "TransferComplete"; id: string; total_bytes: number }
  | { type: "TransferFinalized"; id: string }
  | { type: "TransferError"; id: string; error: string }
  | { type: "ConnectionStatus"; has_remote: boolean }
  | { type: "Ping"; request_id?: string | null }
  | { type: "Pong"; request_id?: string | null }
  | { type: "Error"; request_id?: string | null; message: string };
