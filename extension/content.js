// Waits until the product data is in the page (after any captcha the user
// solves), then hands the HTML to the background script once.
(() => {
  const ext = globalThis.browser ?? globalThis.chrome;
  const POLL_MS = 1500;
  const MAX_WAIT_MS = 3 * 60 * 1000;

  const host = location.hostname;
  const isTemu = host === "temu.com" || host.endsWith(".temu.com");
  const isProduct = isTemu
    ? /-g-\d+\.html$/.test(location.pathname) || new URLSearchParams(location.search).has("goods_id")
    : /MLU-?\d+/i.test(location.pathname);
  if (!isProduct) return;

  const hasData = isTemu
    ? () => [...document.scripts].some((s) => s.textContent.includes("window.rawData"))
    : () => document.querySelector('script[type="application/ld+json"]') !== null;

  const started = Date.now();
  const tick = () => {
    if (hasData()) {
      ext.runtime.sendMessage({
        type: "maeve-page",
        url: location.href,
        html: document.documentElement.outerHTML,
      });
    } else if (Date.now() - started < MAX_WAIT_MS) {
      setTimeout(tick, POLL_MS);
    }
  };
  tick();
})();
