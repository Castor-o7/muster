// The daemon's local token (MUS-7), for a connection on the daemon's own host.
// With TLS on, the gate's local path admits a plain loopback connection only
// with this token; the Rust shim and TUI read the same file. Mirrors
// `muster_entities::pairing::{config_dir, read_local_token}`.
import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

export interface TokenEnv {
  env: Record<string, string | undefined>;
  home: () => string;
  platform: string;
  read: (path: string) => string;
}

const realEnv: TokenEnv = {
  env: process.env,
  home: homedir,
  platform: process.platform,
  read: (path) => readFileSync(path, "utf8"),
};

/** `<config dir>/muster`, the way the Rust side finds it. */
export function configDir(e: TokenEnv = realEnv): string | null {
  const xdg = e.env.XDG_CONFIG_HOME;
  if (xdg) return join(xdg, "muster");
  if (e.platform === "win32" && e.env.APPDATA) return join(e.env.APPDATA, "muster");
  const home = e.env.HOME || e.home();
  return home ? join(home, ".config", "muster") : null;
}

/** The token's first line, or null with no file (a host without a daemon). */
export function readLocalToken(e: TokenEnv = realEnv): string | null {
  const dir = configDir(e);
  if (!dir) return null;
  try {
    const token = e.read(join(dir, "local-token")).split(/\r?\n/)[0]?.trim();
    return token ? token : null;
  } catch {
    return null;
  }
}

const LOOPBACK = new Set(["localhost", "127.0.0.1", "[::1]", "::1"]);

/** Add the local token to a loopback daemon address that has none. A remote
 *  address is left alone: the token admits only connections from this host,
 *  so it must never be sent anywhere else. */
export function withLocalToken(address: string, e: TokenEnv = realEnv): string {
  let url: URL;
  try {
    url = new URL(address);
  } catch {
    return address;
  }
  if (!LOOPBACK.has(url.hostname) || url.searchParams.has("token")) return address;
  const token = readLocalToken(e);
  if (!token) return address;
  url.searchParams.set("token", token);
  return url.toString();
}

