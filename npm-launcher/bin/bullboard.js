#!/usr/bin/env node
/**
 * Thin launcher: prefers cargo-installed `bullboard`, then PATH, then python legacy.
 */
const { spawnSync } = require("child_process");
const path = require("path");
const os = require("os");
const fs = require("fs");

function run(cmd, args) {
  return spawnSync(cmd, args, { stdio: "inherit", env: process.env });
}

const cargoBin = path.join(os.homedir(), ".cargo", "bin", "bullboard");
const candidates = [];
if (fs.existsSync(cargoBin)) candidates.push([cargoBin, []]);
candidates.push(["bullboard", []]);
candidates.push(["python3", ["-m", "bullboard"]]);

for (const [cmd, prefix] of candidates) {
  const r = run(cmd, [...prefix, ...process.argv.slice(2)]);
  if (r.error && r.error.code === "ENOENT") continue;
  process.exit(r.status == null ? 1 : r.status);
}

console.error(
  "bullboard: not found.\n" +
    "Install with:  cargo install --path /path/to/bullboard\n" +
    "           or:  cargo install --git https://github.com/Bull-Inference/bullboard"
);
process.exit(1);
