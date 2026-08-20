import { createResource, For, Show } from 'solid-js';
import { http } from '../api/http';
import { API_BASE } from '../api/auth';

type Listing = { listing_id: number; title: string; starting_price_cents: number };

// The demo backend does not yet filter listings by owner, so this shows all
// listings. (A `/api/v1/me/listings` endpoint is the natural next addition.)
async function fetchListings(): Promise<Listing[]> {
  const r = await http<{ listings: Listing[] }>('GET', `${API_BASE}/api/v1/listings`);
  return r.listings ?? [];
}

export default function MyListings() {
  const [listings] = createResource(fetchListings);
  return (
    <div class="p-6 max-w-2xl mx-auto">
      <h1 class="text-2xl font-bold mb-1">Listings</h1>
      <p class="text-xs text-gray-500 mb-4">(owner filter not yet implemented in the API)</p>
      <Show when={!listings.loading} fallback={<p>Loading…</p>}>
        <For each={listings()} fallback={<p class="text-gray-500">No listings.</p>}>
          {(l) => (
            <div class="border rounded p-3 mb-2">
              <div class="font-semibold">{l.title}</div>
              <div class="text-sm text-gray-600">#{l.listing_id}</div>
            </div>
          )}
        </For>
      </Show>
    </div>
  );
}
