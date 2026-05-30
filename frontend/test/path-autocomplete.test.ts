import { describe, expect, it } from "vitest";
import {
  getRootRelativePath,
  parseAutocompletePath,
  resolveSuggestionBrowsePath,
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

  it("maps exact root inputs back to '.' for root-bounded suggestion fetches", () => {
    expect(getRootRelativePath("/Users/aero.wang", "/Users/aero.wang")).toBe(".");
    expect(resolveSuggestionBrowsePath("/Users/aero.wang", "/Users/aero.wang")).toBe(".");
  });

  it("keeps lookups inside the current root and rejects paths outside it", () => {
    expect(resolveSuggestionBrowsePath("/Users/aero.wang/Documents", "/Users/aero.wang")).toBe(".");
    expect(resolveSuggestionBrowsePath("/Users/other-user", "/Users/aero.wang")).toBeNull();
    expect(resolveSuggestionBrowsePath("/Users/aero.wang/Desk", "/Users/aero.wang")).toBe(".");
  });
});
