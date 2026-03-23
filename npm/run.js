#!/usr/bin/env node

"use strict";

const { spawnSync } = require("child_process");
const path = require("path");

const ext = process.platform === "win32" ? ".exe" : "";
const bin = path.join(__dirname, "bin", `ccost${ext}`);
const result = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });

if (result.error) {
  if (result.error.code === "ENOENT") {
    console.error(
      "ccost binary not found. Try reinstalling:\n  npm install -g ccost"
    );
  } else {
    console.error("Failed to run ccost:", result.error.message);
  }
  process.exit(1);
}

process.exit(result.status ?? 1);
