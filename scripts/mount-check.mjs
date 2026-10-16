/**
 * 面板界面冒烟测试：把**打包产物**真跑一遍，确认界面能渲染出来。
 *
 * 为什么需要它：`curl` 拿到 200 只证明服务器吐了字节。入口包执行报错、
 * 某个 chunk 加载失败、渲染期抛异常 —— 这些都表现为**白屏**，而静态检查
 * 和接口测试全都发现不了。这个脚本抓的就是这一类。
 *
 * jsdom 的 `runScripts` 不执行 `<script type="module">`，所以这里用 jsdom
 * 搭 DOM + Node 的 `vm.SourceTextModule` 亲自加载线上那份 ESM 主包。
 *
 * 用法：
 *   node scripts/mount-check.mjs <baseUrl> [用户名] [密码]
 *
 * 不带账号密码 → 验未登录渲染出登录页；
 * 带账号密码   → 先登录拿 Cookie，再验号池界面渲染。
 */
import vm from "node:vm";
import { CookieJar, JSDOM, VirtualConsole } from "jsdom";

const [base = "http://127.0.0.1:8791", user, pass] = process.argv.slice(2);
const authenticated = Boolean(user && pass);

/** 补 jsdom 缺失、而面板会碰到的浏览器 API。 */
function patchBrowserApis(window) {
  window.matchMedia ??= (query) => ({
    matches: false,
    media: query,
    addEventListener() {},
    removeEventListener() {},
    addListener() {},
    removeListener() {},
    dispatchEvent: () => false,
  });
  window.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  window.scrollTo ??= () => {};
}

/** 登录并取回会话 Cookie（未提供凭据时返回空串）。 */
async function login() {
  if (!authenticated) return "";
  const response = await fetch(`${base}/api/auth/login`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ username: user, password: pass }),
  });
  if (!response.ok) throw new Error(`登录失败：HTTP ${response.status}`);
  const cookies = (response.headers.getSetCookie?.() ?? [])
    .map((entry) => entry.split(";")[0])
    .filter(Boolean);
  if (cookies.length === 0) throw new Error("登录成功但服务端没有下发 Cookie");
  return cookies.join("; ");
}

/** 用 jsdom 建页面，并执行入口 ESM 模块。返回渲染结果与捕获到的异常。 */
async function render(cookie) {
  const pageErrors = [];
  const virtualConsole = new VirtualConsole();
  virtualConsole.on("jsdomError", (error) =>
    pageErrors.push(String(error.message ?? error)),
  );

  const dom = await JSDOM.fromURL(`${base}/`, {
    runScripts: "outside-only",
    pretendToBeVisual: true,
    virtualConsole,
    cookieJar: new CookieJar(),
  });
  const { window } = dom;
  patchBrowserApis(window);
  for (const pair of cookie.split("; ").filter(Boolean)) {
    const [name, ...rest] = pair.split("=");
    window.document.cookie = `${name}=${rest.join("=")}; path=/`;
  }

  const entrySrc = window.document
    .querySelector('script[type="module"]')
    ?.getAttribute("src");
  if (!entrySrc) throw new Error("index.html 里找不到 type=module 的入口脚本");
  const entryUrl = new URL(entrySrc, base).href;

  // 模块缓存：主包与懒加载 chunk 都从这里走。
  const context = vm.createContext(window);
  const cache = new Map();
  const loadModule = async (url) => {
    if (cache.has(url)) return cache.get(url);
    const response = await fetch(url);
    if (!response.ok) throw new Error(`加载 ${url} 失败：HTTP ${response.status}`);
    const module = new vm.SourceTextModule(await response.text(), {
      identifier: url,
      context,
      initializeImportMeta(meta) {
        meta.url = url;
      },
      importModuleDynamically: (specifier, referencing) =>
        loadModule(new URL(specifier, referencing.identifier).href),
    });
    cache.set(url, module);
    return module;
  };

  let fatal = null;
  try {
    const entry = await loadModule(entryUrl);
    await entry.link((specifier, referencing) =>
      loadModule(new URL(specifier, referencing.identifier).href),
    );
    await entry.evaluate({ timeout: 30_000 });
  } catch (error) {
    fatal = error;
  }

  // 等 React 挂载 + 会话探测回来（探测是异步的，界面会先渲染 loading）。
  const root = () => window.document.getElementById("root")?.innerHTML ?? "";
  const deadline = Date.now() + 25_000;
  const want = authenticated ? "账号" : "登录以管理账号池";
  while (!root().includes(want) && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 250));
  }

  return { html: root(), fatal, pageErrors, window };
}

const checks = [];
let fatal = null;
let pageErrors = [];
let html = "";

try {
  const cookie = await login();
  if (authenticated) console.log("  已取得会话 Cookie");
  const result = await render(cookie);
  ({ html, fatal, pageErrors } = result);
} catch (error) {
  fatal = error;
}

if (authenticated) {
  checks.push(
    ["入口模块执行没有抛异常", fatal === null],
    ["界面已渲染", html.length > 200],
    ["已登录：不再显示登录表单", !html.includes("登录以管理账号池")],
    ["显示当前登录用户", html.includes(user)],
    ["号池页签存在", html.includes("账号")],
    ["用量页签存在", html.includes("用量")],
    ["日志页签存在", html.includes("日志")],
    ["设置页签存在", html.includes("设置")],
    ["有添加账号入口", html.includes("添加")],
    ["页面没有未捕获异常", pageErrors.length === 0],
  );
} else {
  checks.push(
    ["入口模块执行没有抛异常", fatal === null],
    ["React 挂载成功", html.length > 0],
    ["渲染出登录页标题「反代号池」", html.includes("反代号池")],
    ["渲染出副标题「登录以管理账号池」", html.includes("登录以管理账号池")],
    ["有账号输入框", html.includes('placeholder="账号"')],
    ["有密码输入框", html.includes('placeholder="密码"')],
    ["有登录按钮", html.includes("登录")],
    ["未登录时看不到号池界面", !html.includes("添加账号")],
    ["页面不是白屏", html.length > 200],
    ["页面没有未捕获异常", pageErrors.length === 0],
  );
}

let failed = 0;
for (const [label, ok] of checks) {
  console.log(`  ${ok ? "\u001b[32m✓\u001b[0m" : "\u001b[31m✗\u001b[0m"} ${label}`);
  if (!ok) failed += 1;
}
if (fatal) console.log(`\n  入口异常：${fatal.stack ?? fatal}`);
if (pageErrors.length) console.log(`\n  页面异常：${pageErrors.join("\n  ")}`);
if (failed && html.length) console.log(`\n  #root 前 600 字：\n  ${html.slice(0, 600)}`);

console.log(
  failed === 0
    ? `\u001b[32m界面冒烟测试通过（${authenticated ? "已登录" : "未登录"}）\u001b[0m`
    : `\u001b[31m${failed} 项失败\u001b[0m`,
);
process.exit(failed === 0 ? 0 : 1);
