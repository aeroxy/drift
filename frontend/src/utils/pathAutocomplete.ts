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

export function getRootRelativePath(path: string, root: string): string | null {
  const normalizedRoot = root !== "/" && root.endsWith("/") ? root.slice(0, -1) : root;
  const rootPrefix = normalizedRoot === "/" ? normalizedRoot : `${normalizedRoot}/`;

  if (path === normalizedRoot) return ".";
  if (path.startsWith(rootPrefix)) return path.slice(rootPrefix.length) || ".";

  return null;
}

export function resolveSuggestionBrowsePath(inputValue: string, root: string): string | null {
  const { parentDir } = parseAutocompletePath(inputValue);
  const relativeParent = getRootRelativePath(parentDir, root);
  const relativeInput = getRootRelativePath(inputValue, root);

  if (relativeParent !== null) return relativeParent;
  if (relativeInput !== null) return ".";

  return null;
}

export function joinPath(dir: string, name: string): string {
  return dir.endsWith("/") ? `${dir}${name}` : `${dir}/${name}`;
}
