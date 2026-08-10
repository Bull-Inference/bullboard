#!/usr/bin/env node
/**
 * Thin launcher: prefers `bullboard` on PATH (pipx/pip), else python -m bullboard.
 */
const { spawnSync } = require("child_process");

function run(cmd, args) {
  return spawnSync(cmd, args, { stdio: "inherit", env: process.env });
}

let r = run("bullboard", process.argv.slice(2));
if (r.error && r.error.code === "ENOENT") {
  r = run("python3", ["-m", "bullboard", ...process.argv.slice(2)]);
}
if (r.error && r.error.code === "ENOENT") {
  console.error(
    "bullboard: Python package not found.\n" +
      "Install with:  pipx install git+https://github.com/Bull-Inference/bullboard.git\n" +
      "           or:  pip install bullboard"
  );
  process.exit(1);
}
process.exit(r.status == null ? 1 : r.status);
