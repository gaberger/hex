import { createResource, createSignal, For, Show } from 'solid-js';
import { A } from '@solidjs/router';
import { http } from '../api/http';
import { API_BASE } from '../api/auth';

type Listing = { listing_id: number; title: string; starting_price_cents: number };

async function fetchListings(): Promise<Listing[]> {
  const r = await http<{ listings: Listing[] }>('GET', `${API_BASE}/api/v1/listings`);
  return r.listings ?? [];
}

const EMOJI: [RegExp, string][] = [
  [/camera|lens|nikon|canon|sony|leica|fuji|polaroid/i, '📷'],
  [/macbook|thinkpad|laptop|xps|framework|razer blade/i, '💻'],
  [/iphone|pixel|galaxy|phone|oneplus|nothing/i, '📱'],
  [/omega|seiko|rolex|casio|tissot|watch|g-shock/i, '⌚'],
  [/fender|gibson|taylor|martin|ibanez|guitar|strat|les paul/i, '🎸'],
  [/dune|sapiens|foundation|lotr|book|edition/i, '📚'],
  [/jordan|yeezy|new balance|dunk|samba|sneaker|nike|adidas/i, '👟'],
  [/trek|specialized|cannondale|brompton|bike/i, '🚲'],
  [/ps5|xbox|switch|steam deck|analogue|console/i, '🎮'],
];
function icon(title: string): string {
  for (const [re, e] of EMOJI) if (re.test(title)) return e;
  return '📦';
}
function condition(title: string): string {
  const m = title.match(/^(Mint|Vintage|Refurbished|Like-New|Rare|Sealed|Pre-Owned|Collector's|Limited-Edition|New|Used)/i);
  return (m ? m[1] : 'Used').toUpperCase();
}
const GRADIENTS = [
  'from-rose-100 to-rose-200', 'from-sky-100 to-sky-200', 'from-amber-100 to-amber-200',
  'from-emerald-100 to-emerald-200', 'from-violet-100 to-violet-200', 'from-cyan-100 to-cyan-200',
  'from-fuchsia-100 to-fuchsia-200', 'from-lime-100 to-lime-200', 'from-orange-100 to-orange-200',
];

export default function Home() {
  const [listings] = createResource(fetchListings);
  const [q, setQ] = createSignal('');
  const shown = () =>
    (listings() ?? []).filter((l) => l.title.toLowerCase().includes(q().toLowerCase()));

  return (
    <div class="min-h-screen bg-gray-50 text-gray-900">
      <header class="sticky top-0 z-10 bg-white border-b border-gray-200">
        <div class="max-w-6xl mx-auto px-4 py-3 flex items-center gap-4">
          <A href="/" class="text-2xl font-extrabold tracking-tight shrink-0">
            <span class="text-blue-600">hex</span><span class="text-amber-500">Bay</span>
          </A>
          <div class="flex-1">
            <input
              value={q()}
              onInput={(e) => setQ(e.currentTarget.value)}
              placeholder="Search for anything"
              class="w-full border border-gray-300 rounded-full px-4 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-400"
            />
          </div>
          <nav class="hidden sm:flex items-center gap-4 text-sm font-medium text-gray-700 shrink-0">
            <A href="/post-listing" class="hover:text-blue-600">Sell</A>
            <A href="/my-bids" class="hover:text-blue-600">My bids</A>
            <A href="/my-won" class="hover:text-blue-600">Won</A>
            <A href="/register" class="bg-blue-600 text-white px-3 py-1.5 rounded-full hover:bg-blue-700">Sign up</A>
          </nav>
        </div>
      </header>

      <main class="max-w-6xl mx-auto px-4 py-6">
        <h1 class="text-lg font-semibold text-gray-700 mb-4">
          {shown().length} {shown().length === 1 ? 'result' : 'results'}
        </h1>

        <Show when={!listings.loading} fallback={<p class="text-gray-500">Loading…</p>}>
          <Show when={listings.error}>
            <p class="text-red-600">Couldn't reach the API at {API_BASE} — is the backend running?</p>
          </Show>
          <div class="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 gap-4">
            <For each={shown()}>
              {(l) => (
                <A
                  href="/my-bids"
                  class="group bg-white rounded-xl border border-gray-200 overflow-hidden hover:shadow-lg transition-shadow"
                >
                  <div class={`h-40 flex items-center justify-center text-6xl bg-gradient-to-br ${GRADIENTS[l.listing_id % GRADIENTS.length]}`}>
                    {icon(l.title)}
                  </div>
                  <div class="p-3">
                    <span class="inline-block text-[10px] font-semibold tracking-wide bg-gray-100 text-gray-600 rounded px-1.5 py-0.5 mb-1">
                      {condition(l.title)}
                    </span>
                    <div class="text-sm text-gray-800 line-clamp-2 h-10 group-hover:text-blue-600">{l.title}</div>
                    <div class="mt-1 text-lg font-bold">${(l.starting_price_cents / 100).toFixed(2)}</div>
                    <div class="text-xs text-gray-500">or Best Offer</div>
                  </div>
                </A>
              )}
            </For>
          </div>
          <Show when={shown().length === 0 && !listings.error}>
            <p class="text-gray-500">No items match “{q()}”.</p>
          </Show>
        </Show>
      </main>
    </div>
  );
}
