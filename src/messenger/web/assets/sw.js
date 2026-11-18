const CACHE_NAME = "tuner-dash-static-v9";
const STATIC_ASSETS = [
  "/dashboard/assets/dashboard.css",
  "/dashboard/assets/app.js",
  "/dashboard/assets/i18n.js",
  "/dashboard/assets/chat.js",
  "/dashboard/assets/slash.js",
  "/dashboard/assets/inspector.js",
  "/dashboard/assets/onboarding.js",
  "/dashboard/assets/markdown.js",
  "/dashboard/manifest.webmanifest",
  "/dashboard/icon.svg"
];

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches.open(CACHE_NAME).then((cache) => cache.addAll(STATIC_ASSETS)).then(() => self.skipWaiting())
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches.keys().then((keys) =>
      Promise.all(keys.filter((k) => k !== CACHE_NAME).map((k) => caches.delete(k)))
    ).then(() => self.clients.claim())
  );
});

self.addEventListener("fetch", (event) => {
  const url = new URL(event.request.url);
  if (event.request.method !== "GET") return;
  if (
    url.pathname.startsWith("/dashboard/api/") ||
    url.pathname.startsWith("/dashboard/magic/") ||
    url.pathname === "/dashboard" ||
    url.pathname === "/dashboard/login"
  ) {
    return;
  }
  if (
    url.pathname.startsWith("/dashboard/assets/") ||
    url.pathname === "/dashboard/icon.svg" ||
    url.pathname === "/dashboard/manifest.webmanifest"
  ) {
    event.respondWith(
      fetch(event.request)
        .then((res) => {
          if (res.ok) {
            const copy = res.clone();
            caches.open(CACHE_NAME).then((cache) => cache.put(event.request, copy));
          }
          return res;
        })
        .catch(() => caches.match(event.request))
    );
  }
});
