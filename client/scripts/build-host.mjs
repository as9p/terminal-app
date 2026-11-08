// Builds term-host in release and puts it where bundle.externalBin looks for it.
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const clientDir = dirname(dirname(fileURLToPath(import.meta.url)));
const rootDir = dirname(clientDir);

const rustc = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
const hostTriple = /^host: (.+)$/m.exec(rustc)[1];
const triple = process.env.TAURI_ENV_TARGET_TRIPLE ?? hostTriple;
const cross = triple !== hostTriple;

const crossArgs = cross ? ["--target", triple] : [];
execFileSync("cargo", ["build", "--release", "-p", "term-host", ...crossArgs], {
  cwd: rootDir,
  stdio: "inherit",
});

const suffix = triple.includes("windows") ? ".exe" : "";
const targetDir = process.env.CARGO_TARGET_DIR ?? join(rootDir, "target");
const built = join(targetDir, ...(cross ? [triple] : []), "release", `term-host${suffix}`);
const sidecar = join(clientDir, "src-tauri", "binaries", `term-host-${triple}${suffix}`);

mkdirSync(dirname(sidecar), { recursive: true });
// Removed first: macOS can kill a signed program whose file is rewritten in place.
rmSync(sidecar, { force: true });
copyFileSync(built, sidecar);
