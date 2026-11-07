import { describe, expect, test } from "bun:test";

import { withMykoPath } from "../src/daemon.js";
import { readLocalToken, withLocalToken, type TokenEnv } from "../src/local-token.js";

// With TLS on, the daemon's local gate admits a loopback connection only with
// the host's local token (MUS-7); the Rust shim and TUI read the same file.
const env = (files: Record<string, string>, vars: Record<string, string | undefined> = {}): TokenEnv => ({
  env: { HOME: "/h", ...vars },
  home: () => "/h",
  platform: "linux",
  read: (path) => {
    if (path in files) return files[path];
    throw new Error("ENOENT");
  },
});
const withToken = env({ "/h/.config/muster/local-token": "abc123\n" });

describe("the local token", () => {
  test("is added to a loopback address", () => {
    expect(withLocalToken("ws://localhost:6255", withToken)).toBe("ws://localhost:6255/?token=abc123");
    expect(withLocalToken("ws://127.0.0.1:6255/myko", withToken)).toBe("ws://127.0.0.1:6255/myko?token=abc123");
  });
  test("is never sent to another machine", () => {
    expect(withLocalToken("ws://murph.lan:6255", withToken)).toBe("ws://murph.lan:6255");
  });
  test("leaves an address that has one, or a host with no daemon, alone", () => {
    expect(withLocalToken("ws://localhost:6255?token=x", withToken)).toBe("ws://localhost:6255?token=x");
    expect(withLocalToken("ws://localhost:6255", env({}))).toBe("ws://localhost:6255");
  });
  test("is found under XDG_CONFIG_HOME like the Rust side", () => {
    expect(readLocalToken(env({ "/x/muster/local-token": "t\n" }, { XDG_CONFIG_HOME: "/x" }))).toBe("t");
  });

  test("composes with the /myko path from the default address", () => {
    // What daemonUrl does, with an injected token file.
    for (const address of ["ws://localhost:6255", "ws://localhost:6255/", "ws://127.0.0.1:6255/myko"]) {
      const url = new URL(withLocalToken(withMykoPath(address), withToken));
      expect(url.pathname).toBe("/myko");
      expect(url.searchParams.get("token")).toBe("abc123");
    }
  });
});
