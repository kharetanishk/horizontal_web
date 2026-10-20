import type { NextConfig } from "next";
import { PHASE_DEVELOPMENT_SERVER } from "next/constants";

// Production is a static export served by the Rust API. Rewrites do not work
// with `output: "export"`, so the /api proxy exists only under `next dev`.
export default function config(phase: string): NextConfig {
  if (phase === PHASE_DEVELOPMENT_SERVER) {
    return {
      rewrites: async () => [{ source: "/api/:path*", destination: "http://localhost:8080/api/:path*" }],
    };
  }
  return { output: "export" };
}
