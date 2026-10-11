// Drive the mock in a real browser and click the switcher for real.
// Prints what the DOM holds after each click, so a broken switcher
// proves itself instead of being argued about.
// Usage: node drive.js [url]
const { spawn } = require("node:child_process");
const http = require("node:http");

const URL = process.argv[2] || "http://127.0.0.1:8740/index.html";
const PORT = 9222;

const get = (path) =>
  new Promise((res, rej) => {
    http.get({ host: "127.0.0.1", port: PORT, path }, (r) => {
      let d = "";
      r.on("data", (c) => (d += c));
      r.on("end", () => res(JSON.parse(d)));
    }).on("error", rej);
  });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

(async () => {
  const chrome = spawn("chromium", [
    "--headless=new", "--disable-gpu", "--no-sandbox", "--hide-scrollbars",
    `--remote-debugging-port=${PORT}`, "--window-size=1440,900",
    "--user-data-dir=/tmp/cdp-profile", URL,
  ], { stdio: "ignore" });

  try {
    let targets = null;
    for (let i = 0; i < 40; i++) {
      try { targets = await get("/json/list"); break; } catch { await sleep(250); }
    }
    if (!targets) throw new Error("chromium never came up");
    const page = targets.find((t) => t.type === "page" && t.webSocketDebuggerUrl);
    const ws = new (require("node:worker_threads"), globalThis.WebSocket)(page.webSocketDebuggerUrl);

    await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
    let id = 0;
    const send = (method, params = {}) =>
      new Promise((res) => {
        const myId = ++id;
        const onMsg = (ev) => {
          const m = JSON.parse(ev.data);
          if (m.id === myId) { ws.removeEventListener("message", onMsg); res(m); }
        };
        ws.addEventListener("message", onMsg);
        ws.send(JSON.stringify({ id: myId, method, params }));
      });

    const evalJs = async (expr) => {
      const r = await send("Runtime.evaluate", {
        expression: expr, returnByValue: true, awaitPromise: true,
      });
      return r.result?.result?.value;
    };

    await send("Page.enable");
    await send("Runtime.enable");
    await sleep(1200);

    const report = async (label) => {
      const boards = await evalJs(`
        [...document.querySelectorAll('.board')].map(b => b.className.replace('board ','')).join(',')
      `);
      const pressed = await evalJs(`
        [...document.querySelectorAll('.switch button')]
          .filter(b => b.getAttribute('aria-pressed') === 'true')
          .map(b => b.textContent.trim()).join(',')
      `);
      const tag = await evalJs(`
        [...document.querySelectorAll('.board')]
          .filter(b => b.style.display !== 'none')
          .map(b => b.querySelector('.tag b')?.textContent).join(',') || 'none'
      `);
      console.log(`${label}: boards=[${boards}] pressed=[${pressed}] visible=${tag}`);
    };

    await report("on load");

    for (const [name, i] of [["A", 0], ["B", 1], ["C", 2], ["A again", 0]]) {
      await evalJs(`document.querySelectorAll('.switch button')[${i}].click()`);
      await sleep(500);
      await report(`click ${name}`);
    }

    // Keys too, since the README advertises 1/2/3.
    for (const k of ["2", "3"]) {
      await send("Input.dispatchKeyEvent", { type: "keyDown", key: k, text: k });
      await send("Input.dispatchKeyEvent", { type: "keyUp", key: k });
      await sleep(500);
      await report(`key ${k}`);
    }
  } finally {
    chrome.kill("SIGKILL");
  }
})().catch((e) => { console.error("FAILED:", e.message); process.exit(1); });
