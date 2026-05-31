# Address Bar Browsing

The address bar in drift's web UI provides autocomplete suggestions for both local and remote directories, making it easy to navigate without clicking through the file tree.

## Features

- **Autocomplete suggestions** — type a path and get suggestions for directories
- **Local and remote support** — works for both panes in the two-pane browser
- **Cached suggestions** — uses cached entries when browsing the current directory
- **REST-based remote browsing** — fetches remote suggestions via `/api/browse-remote` without WebSocket side effects
- **Path normalization** — handles absolute paths correctly relative to root_dir

## How it works

### Local suggestions

When typing in the local address bar:

1. If the parent directory matches the current working directory, suggestions come from cached `localEntries`
2. Otherwise, the frontend calls `/api/browse?path=<relative_parent>` to fetch directory entries
3. Absolute paths are normalized relative to `root_dir` (obtained from `/api/info`)

### Remote suggestions

When typing in the remote address bar:

1. If the parent directory matches the current remote working directory, suggestions come from cached `remoteEntries`
2. Otherwise, the frontend calls `/api/browse-remote?path=<relative_parent>` to fetch remote directory entries via REST
3. This avoids sending `BrowseRequest` messages over WebSocket, preventing side effects on the remote panel state

### Path normalization

The frontend maintains `localRootDirRef` and `remoteRootDirRef` to track the root directories. When an absolute path is entered:

- `getRootRelativePath()` converts absolute paths to relative paths
- `resolveSuggestionBrowsePath()` determines the correct parent directory for suggestions
- Paths outside the root directory are rejected

## REST API

### `GET /api/browse-remote`

Fetches directory entries from the connected remote server via REST (no WebSocket side effects).

**Parameters:**
- `path` (query, optional) — directory path to list (default: `"."`)

**Response:**
```json
{
  "hostname": "remote-host",
  "cwd": "/path/to/dir",
  "entries": [
    {
      "name": "Documents",
      "is_dir": true,
      "size": 4096,
      "modified": 1717200000
    }
  ]
}
```

**Errors:**
- `400 Bad Request` — no remote connection exists
- `502 Bad Gateway` — remote server returned an error
- `504 Gateway Timeout` — remote server did not respond within 10 seconds

## Implementation details

### Frontend utilities

The `frontend/src/utils/pathAutocomplete.ts` module provides helper functions:

- `parseAutocompletePath(inputValue)` — extracts parent directory and prefix from input
- `shouldUseCachedSuggestions(inputValue, cwd)` — determines if cached entries can be used
- `getRootRelativePath(path, root)` — converts absolute path to root-relative
- `resolveSuggestionBrowsePath(inputValue, root)` — determines the browse path for suggestions
- `joinPath(dir, name)` — joins directory and filename with proper slash handling

### Backend endpoint

The `/api/browse-remote` endpoint in `src/server/file_api.rs`:

1. Generates a unique `request_id` for correlation
2. Sends a `BrowseRequest` to the remote server via the existing WebSocket connection
3. Registers a pending response channel with timeout
4. Returns the remote response as JSON

This leverages the v3 protocol's request/response correlation IDs to match responses correctly, even with concurrent requests.

## Testing

Integration tests are in `frontend/test/address-bar-browse.test.ts`:

- `/api/browse-remote` endpoint tests
- `/api/browse` path normalization tests
- Cached suggestion behavior tests
- WebSocket `BrowseRequest` navigation tests