import { describe, expect, it } from "vitest";
import {
  parseAutocompletePath,
  shouldUseCachedSuggestions,
} from "../src/utils/pathAutocomplete";

describe("path autocomplete helpers", () => {
  it("parses parent dir and prefix for an exact cwd input without trailing slash", () => {
    expect(parseAutocompletePath("/home/user")).toEqual({
      parentDir: "/home",
      prefix: "user",
    });
  });

  it("does not reuse cached entries when the input matches cwd exactly", () => {
    expect(shouldUseCachedSuggestions("/home/user", "/home/user")).toBe(false);
    expect(shouldUseCachedSuggestions("/remote/work", "/remote/work")).toBe(false);
  });

  it("reuses cached entries when browsing inside the current cwd", () => {
    expect(shouldUseCachedSuggestions("/home/user/", "/home/user")).toBe(true);
    expect(shouldUseCachedSuggestions("/home/user/do", "/home/user")).toBe(true);
    expect(shouldUseCachedSuggestions("/remote/work/pro", "/remote/work")).toBe(true);
  });
});
