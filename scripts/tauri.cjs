/**
 * tauri CLI 的包装：启动前删掉 CARGO_TARGET_DIR。
 *
 * 为什么：这台机器的用户级环境变量把 cargo 产物指向了共享目录
 * （E:\CodexCaches\cargo-target\git-kline-market），dev 跑的是旧编译产物，
 * "改了代码重启还是旧行为"就是它干的。项目内 .cargo/config.toml 的
 * target-dir 会被这个 env 压住（cargo 规则：env 优先），所以只能在
 * 进程层把它删掉。cross-env 设空串会让 cargo 报错，所以用 node 删。
 */
const path = require("node:path");
const { spawn } = require("node:child_process");

delete process.env.CARGO_TARGET_DIR;

// 同时剔除 PATH 里的 Git usr/bin：那里的 link.exe 是 GNU coreutils 的"创建硬链接"，
// 会劫持 MSVC 链接器，release 链接时 rustc 随机崩（ACCESS_VIOLATION / STACK_BUFFER_OVERRUN）。
// 该目录同时不在 pnpm 的 PATH 注入范围里，删掉无副作用。
const pathSuffix = path.join("git", "usr", "bin").toLowerCase();
process.env.PATH = process.env.PATH.split(";")
  .filter((part) => part && !part.toLowerCase().replace(/\+$/, "").endsWith(pathSuffix))
  .join(";");

// tauri CLI 平时由 pnpm run 注入 PATH；直接 node 调用时要显式补上。
const localBin = path.resolve(__dirname, "..", "node_modules", ".bin");
if (!process.env.PATH.split(";").some((part) => part.toLowerCase() === localBin.toLowerCase())) {
  process.env.PATH = `${localBin};${process.env.PATH}`;
}

const result = spawn("tauri", process.argv.slice(2), {
  stdio: "inherit",
  shell: process.platform === "win32",
});
result.on("exit", (code) => process.exit(code ?? 0));
