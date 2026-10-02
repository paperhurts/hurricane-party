// Where the player loads a file from (D184). The asset protocol on Windows;
// on Linux the loopback media server, since WebKitGTK plays through
// GStreamer and GStreamer cannot fetch from a custom scheme. Rust says which
// once, at the first call, and every window asks the same.
import { convertFileSrc, invoke } from "@tauri-apps/api/core";

let base: Promise<string | null> | null = null;

/** The URL a media element plays `path` from. */
export async function mediaSrc(path: string): Promise<string> {
  base ??= invoke<string | null>("media_base").catch(() => null);
  const b = await base;
  return b ? `${b}/${encodeURIComponent(path)}` : convertFileSrc(path);
}
