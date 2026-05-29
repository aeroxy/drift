export interface ParsedAutocompletePath {
  parentDir: string;
  prefix: string;
}

export function parseAutocompletePath(inputValue: string): ParsedAutocompletePath {
  const lastSlash = inputValue.lastIndexOf("/");
  return {
    parentDir: lastSlash > 0 ? inputValue.slice(0, lastSlash) : "/",
    prefix: inputValue.slice(lastSlash + 1).toLowerCase(),
  };
}

export function shouldUseCachedSuggestions(inputValue: string, cwd: string): boolean {
  return parseAutocompletePath(inputValue).parentDir === cwd;
}
