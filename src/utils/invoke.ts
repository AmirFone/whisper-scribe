import { invoke, type InvokeArgs } from "@tauri-apps/api/core";

// Thin wrapper around Tauri's `invoke` that logs failures with the command
// name instead of letting them vanish into a `catch (_) {}`. Callers that
// need the real error can still import `invoke` directly.
//
// Returns `null` on failure so callers can pattern-match without another
// try/catch. Logging goes to the browser console — in a packaged macOS
// bundle this lands in Safari Web Inspector when attached.
export async function tryInvoke<T>(cmd: string, args?: InvokeArgs): Promise<T | null> {
  try {
    return await invoke<T>(cmd, args);
  } catch (err) {
    console.warn("[ipc]", cmd, err);
    return null;
  }
}
