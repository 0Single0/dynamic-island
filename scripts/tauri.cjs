/**
 * tauri CLI 的包装：启动前删掉 CARGO_TARGET_DIR。
 *
 * 为什么：这台机器的用户级环境变量把 cargo 产物指向了共享目录
 * （E:\CodexCaches\cargo-target\git-kline-market），dev 跑的是旧编译产物，
 * "改了代码重启还是旧行为"就是它干的。项目内 .cargo/config.toml 的
 * target-dir 会被这个 env 压住（cargo 规则：env 优先），所以只能在
 * 进程层把它删掉。cross-env 设空串会让 cargo 报错，所以用 node 删。
 */
const { spawn } = require("node:child_process");

delete process.env.CARGO_TARGET_DIR;

const result = spawn("tauri", process.argv.slice(2), {
  stdio: "inherit",
  shell: process.platform === "win32",
});
result.on("exit", (code) => process.exit(code ?? 0));
