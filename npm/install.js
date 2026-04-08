"use strict";

const https = require("https");
const http = require("http");
const fs = require("fs");
const path = require("path");
const { execSync } = require("child_process");
const os = require("os");

if (process.env.CCOST_SKIP_INSTALL) {
  console.log("Skipping ccost binary download (CCOST_SKIP_INSTALL is set)");
  process.exit(0);
}

const VERSION = require("./package.json").version;
const REPO = "cc-friend/ccost";

function getPlatformName() {
  const platform = process.platform;
  const arch = process.arch;

  if (platform === "darwin") {
    if (arch === "x64") return "darwin-x64";
    if (arch === "arm64") return "darwin-arm64";
  }

  if (platform === "win32") {
    if (arch === "x64") return "win32-x64";
    if (arch === "ia32") return "win32-ia32";
    if (arch === "arm64") return "win32-arm64";
  }

  if (platform === "linux") {
    const musl = isMusl();
    if (arch === "x64") return musl ? "linux-x64-musl" : "linux-x64";
    if (arch === "arm64") return musl ? "linux-arm64-musl" : "linux-arm64";
    if (arch === "ia32") return "linux-ia32";
    if (arch === "arm") return "linux-arm";
  }

  return null;
}

function isMusl() {
  try {
    if (fs.existsSync("/etc/alpine-release")) return true;
  } catch {}

  try {
    const output = execSync("ldd --version 2>&1", { encoding: "utf8" });
    if (output.toLowerCase().includes("musl")) return true;
  } catch (e) {
    const out = [e.stdout, e.stderr].filter(Boolean).join("").toLowerCase();
    if (out.includes("musl")) return true;
  }

  return false;
}

function download(url) {
  return new Promise((resolve, reject) => {
    const client = url.startsWith("https") ? https : http;
    client
      .get(url, { headers: { "User-Agent": "ccost-npm" } }, (res) => {
        if (
          res.statusCode >= 300 &&
          res.statusCode < 400 &&
          res.headers.location
        ) {
          res.resume();
          download(res.headers.location).then(resolve, reject);
          return;
        }
        if (res.statusCode !== 200) {
          res.resume();
          reject(new Error(`HTTP ${res.statusCode} from ${url}`));
          return;
        }
        const chunks = [];
        res.on("data", (chunk) => chunks.push(chunk));
        res.on("end", () => resolve(Buffer.concat(chunks)));
        res.on("error", reject);
      })
      .on("error", reject);
  });
}

async function main() {
  const platformName = getPlatformName();
  if (!platformName) {
    console.error(
      `Unsupported platform: ${process.platform}-${process.arch}\n` +
        "Install from source: cargo install ccost"
    );
    process.exit(1);
  }

  const isWindows = process.platform === "win32";
  const ext = isWindows ? ".zip" : ".tar.gz";
  const url = `https://github.com/${REPO}/releases/download/v${VERSION}/ccost-${platformName}${ext}`;

  console.log(`Downloading ccost v${VERSION} for ${platformName}...`);

  let data;
  try {
    data = await download(url);
  } catch (err) {
    console.error(
      `Failed to download from ${url}\n${err.message}\n` +
        "Install from source: cargo install ccost"
    );
    process.exit(1);
  }

  const binDir = path.join(__dirname, "bin");
  fs.mkdirSync(binDir, { recursive: true });

  const tmpFile = path.join(os.tmpdir(), `ccost-${Date.now()}${ext}`);
  fs.writeFileSync(tmpFile, data);

  try {
    if (isWindows) {
      execSync(`tar -xf "${tmpFile}" -C "${binDir}"`, { stdio: "ignore" });
    } else {
      execSync(`tar xzf "${tmpFile}" -C "${binDir}"`, { stdio: "ignore" });
      fs.chmodSync(path.join(binDir, "ccost"), 0o755);
    }
  } finally {
    try {
      fs.unlinkSync(tmpFile);
    } catch {}
  }

  console.log(`ccost v${VERSION} installed successfully.`);
}

main().catch((err) => {
  console.error("Failed to install ccost:", err.message);
  process.exit(1);
});
