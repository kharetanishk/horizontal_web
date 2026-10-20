import { expect, test } from "vitest";
import { PHASE_DEVELOPMENT_SERVER, PHASE_PRODUCTION_BUILD } from "next/constants";
import config from "./next.config";

test("production build is a static export", () => {
  expect(config(PHASE_PRODUCTION_BUILD).output).toBe("export");
});

test("dev server proxies /api to the Rust API", async () => {
  const rewrites = await config(PHASE_DEVELOPMENT_SERVER).rewrites!();
  expect(rewrites).toEqual([{ source: "/api/:path*", destination: "http://localhost:8080/api/:path*" }]);
});
