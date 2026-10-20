import { expect, test } from "vitest";
import type { Anchor, ApiErrorBody } from "@nodemind/schemas";

test("generated wire types are importable from @nodemind/schemas", () => {
  const anchor: Anchor = { kind: "code", start: 0, end: 10, start_line: 1, end_line: 2, text: "x" };
  const body: ApiErrorBody = { error: { code: "not_found", message: "Canvas not found" } };
  expect([anchor.kind, body.error.code]).toEqual(["code", "not_found"]);
});
