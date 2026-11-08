// Placeholder until the terminal view replaces it. Prints whether a token came back, never the token.
import { invoke } from "@tauri-apps/api/core";

type HostConnection = { port: number; token: string };

async function main(): Promise<void> {
  const status = document.querySelector("#status");
  if (!status) return;
  try {
    const { port, token } = await invoke<HostConnection>("host_connection");
    status.textContent = `Host on port ${port}, token ${token ? "received" : "missing"}`;
  } catch (error) {
    status.textContent = `Could not reach the host: ${String(error)}`;
  }
}

void main();
