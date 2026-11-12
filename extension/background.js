// Forwards pages to the Maeve app. Content scripts cannot reach localhost
// themselves; this script can through its host permission.
const ext = globalThis.browser ?? globalThis.chrome;
const ENDPOINT = "http://127.0.0.1:47821/page";

function badge(tabId, text, color) {
  ext.action.setBadgeText({ tabId, text });
  if (color) ext.action.setBadgeBackgroundColor({ tabId, color });
}

ext.runtime.onMessage.addListener((msg, sender) => {
  const tabId = sender.tab?.id;
  if (msg?.type !== "maeve-page" || tabId === undefined) return;
  fetch(ENDPOINT, {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-Maeve-Extension": "1" },
    body: JSON.stringify({ url: msg.url, html: msg.html }),
  })
    .then((res) => badge(tabId, res.ok ? "✓" : "!", res.ok ? "#4f7a4a" : "#b4543a"))
    .catch(() => badge(tabId, "", null));
});
