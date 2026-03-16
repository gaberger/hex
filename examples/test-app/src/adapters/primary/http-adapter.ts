// Primary adapter — HTTP server serving web UI
// Imports only from ports and use cases
import type { IHttpServer } from "../../core/ports/index.js";
import type { WeatherService } from "../../core/usecases/weather-service.js";

export class HttpAdapter implements IHttpServer {
  private server: ReturnType<typeof Bun.serve> | null = null;

  constructor(private readonly weatherService: WeatherService) {}

  async start(port: number): Promise<void> {
    const service = this.weatherService;

    this.server = Bun.serve({
      port,
      async fetch(req) {
        const url = new URL(req.url);

        if (url.pathname === "/api/weather" && req.method === "GET") {
          const city = url.searchParams.get("city");
          if (!city) return Response.json({ error: "city parameter required" }, { status: 400 });
          try {
            const weather = await service.getWeather(city);
            return Response.json(weather);
          } catch (e) {
            return Response.json({ error: (e as Error).message }, { status: 502 });
          }
        }

        if (url.pathname === "/api/favorites" && req.method === "GET") {
          const favorites = await service.getFavorites();
          return Response.json(favorites);
        }

        if (url.pathname === "/api/favorites" && req.method === "POST") {
          const body = await req.json();
          await service.addFavorite(body.city, body.country);
          return Response.json({ ok: true }, { status: 201 });
        }

        if (url.pathname === "/api/favorites" && req.method === "DELETE") {
          const id = url.searchParams.get("id");
          if (!id) return Response.json({ error: "id parameter required" }, { status: 400 });
          await service.removeFavorite(id);
          return Response.json({ ok: true });
        }

        if (url.pathname === "/api/favorites/weather" && req.method === "GET") {
          const weather = await service.getFavoritesWeather();
          return Response.json(weather);
        }

        if (url.pathname === "/" || url.pathname === "/index.html") {
          return new Response(HTML, { headers: { "Content-Type": "text/html" } });
        }

        return Response.json({ error: "Not found" }, { status: 404 });
      },
    });

    console.log(`Weather app running at http://localhost:${port}`);
  }

  async stop(): Promise<void> {
    this.server?.stop();
    this.server = null;
  }
}

const HTML = `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Weather App</title>
  <style>
    * { margin: 0; padding: 0; box-sizing: border-box; }
    body { font-family: system-ui, sans-serif; background: #0f172a; color: #e2e8f0; min-height: 100vh; padding: 2rem; }
    .container { max-width: 640px; margin: 0 auto; }
    h1 { text-align: center; margin-bottom: 1.5rem; font-size: 1.5rem; }
    .search { display: flex; gap: 0.5rem; margin-bottom: 1.5rem; }
    input { flex: 1; padding: 0.75rem; border-radius: 0.5rem; border: 1px solid #334155; background: #1e293b; color: #e2e8f0; font-size: 1rem; }
    button { padding: 0.75rem 1.25rem; border-radius: 0.5rem; border: none; background: #3b82f6; color: white; cursor: pointer; font-size: 1rem; }
    button:hover { background: #2563eb; }
    .card { background: #1e293b; border-radius: 0.75rem; padding: 1.25rem; margin-bottom: 1rem; }
    .weather-main { display: flex; align-items: center; gap: 1rem; }
    .temp { font-size: 2.5rem; font-weight: bold; }
    .details { display: grid; grid-template-columns: 1fr 1fr; gap: 0.5rem; margin-top: 1rem; font-size: 0.9rem; color: #94a3b8; }
    .fav-btn { background: none; border: none; font-size: 1.5rem; cursor: pointer; padding: 0.25rem; color: #e2e8f0; }
    .section-title { font-size: 0.85rem; color: #64748b; text-transform: uppercase; letter-spacing: 0.05em; margin: 1.5rem 0 0.75rem; }
    .favorites-list .card { padding: 0.75rem 1rem; display: flex; justify-content: space-between; align-items: center; }
    .error { color: #f87171; text-align: center; padding: 1rem; }
    .loading { text-align: center; color: #64748b; padding: 1rem; }
  </style>
</head>
<body>
  <div class="container">
    <h1>Weather</h1>
    <div class="search">
      <input id="city" type="text" placeholder="Search city..." />
      <button onclick="search()">Search</button>
    </div>
    <div id="result"></div>
    <div class="section-title">Favorites</div>
    <div id="favorites" class="favorites-list"></div>
  </div>
  <script>
    const $ = (id) => document.getElementById(id);
    const input = $("city");
    input.addEventListener("keydown", (e) => { if (e.key === "Enter") search(); });

    function el(tag, attrs, ...children) {
      const e = document.createElement(tag);
      if (attrs) Object.entries(attrs).forEach(([k, v]) => {
        if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
        else if (k === "className") e.className = v;
        else e.setAttribute(k, v);
      });
      children.forEach(c => {
        if (typeof c === "string") e.appendChild(document.createTextNode(c));
        else if (c) e.appendChild(c);
      });
      return e;
    }

    async function search() {
      const city = input.value.trim();
      if (!city) return;
      const result = $("result");
      result.replaceChildren(el("div", { className: "loading" }, "Loading..."));
      try {
        const res = await fetch("/api/weather?city=" + encodeURIComponent(city));
        const data = await res.json();
        if (!res.ok) throw new Error(data.error);
        result.replaceChildren(weatherCard(data));
      } catch (e) {
        result.replaceChildren(el("div", { className: "error" }, e.message));
      }
    }

    function weatherCard(w) {
      const img = el("img", { src: "https://openweathermap.org/img/wn/" + w.icon + "@2x.png", width: "64" });
      const favBtn = el("button", {
        className: "fav-btn",
        title: "Add to favorites",
        onclick: () => addFav(w.location.city, w.location.country)
      }, String.fromCodePoint(0x2606));

      return el("div", { className: "card" },
        el("div", { className: "weather-main" },
          img,
          el("div", null,
            el("div", { className: "temp" }, Math.round(w.temperature) + String.fromCodePoint(0x00B0) + "C"),
            el("div", null, w.description)
          ),
          favBtn
        ),
        el("div", { style: "font-size:0.85rem;color:#94a3b8;margin-top:0.25rem" },
          w.location.city + ", " + w.location.country),
        el("div", { className: "details" },
          el("div", null, "Feels like: " + Math.round(w.feelsLike) + String.fromCodePoint(0x00B0) + "C"),
          el("div", null, "Humidity: " + w.humidity + "%"),
          el("div", null, "Wind: " + w.windSpeed + " m/s")
        )
      );
    }

    async function addFav(city, country) {
      await fetch("/api/favorites", { method: "POST", headers: {"Content-Type":"application/json"}, body: JSON.stringify({city, country}) });
      loadFavorites();
    }

    async function removeFav(id) {
      await fetch("/api/favorites?id=" + encodeURIComponent(id), { method: "DELETE" });
      loadFavorites();
    }

    async function loadFavorites() {
      try {
        const res = await fetch("/api/favorites");
        const favs = await res.json();
        const container = $("favorites");
        if (!favs.length) {
          container.replaceChildren(el("div", { className: "card", style: "color:#64748b" }, "No favorites yet"));
          return;
        }
        container.replaceChildren(...favs.map(f =>
          el("div", { className: "card" },
            el("span", null, f.city + ", " + f.country),
            el("button", {
              className: "fav-btn",
              title: "Remove",
              onclick: () => removeFav(f.id)
            }, String.fromCodePoint(0x2715))
          )
        ));
      } catch(e) { $("favorites").replaceChildren(); }
    }

    loadFavorites();
  </script>
</body>
</html>`;
