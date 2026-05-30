import { createResource, For, Show } from 'solid-js';
import { A } from '@solidjs/router';
import { http } from '../api/http';
import { API_BASE } from '../api/auth';

type Listing = { listing_id: number; title: string; starting_price_cents: number };

async function fetchListings(): Promise<Listing[]> {
  const r = await http<{ listings: Listing[] }>('GET', `${API_BASE}/api/v1/listings`);
  return r.listings ?? [];
}

export default function Home() {
  const [listings] = createResource(fetchListings);
  return (
    <div class="p-6 max-w-2xl mx-auto">
      <h1 class="text-2xl font-bold mb-4">Listings</h1>
      <nav class="mb-6 space-x-4 text-blue-600">
        <A href="/post-listing">+ Post a listing</A>
        <A href="/register">Register</A>
        <A href="/my-bids">Place a bid</A>
        <A href="/my-won">My won items</A>
      </nav>
      <Show when={!listings.loading} fallback={<p>Loading…</p>}>
        <For each={listings()} fallback={<p class="text-gray-500">No listings yet.</p>}>
          {(l) => (
            <div class="border rounded p-3 mb-2">
              <div class="font-semibold">{l.title}</div>
              <div class="text-sm text-gray-600">
                #{l.listing_id} — starting ${(l.starting_price_cents / 100).toFixed(2)}
              </div>
            </div>
          )}
        </For>
      </Show>
      <Show when={listings.error}>
        <p class="text-red-600">Failed to load listings (is the backend running on {API_BASE}?)</p>
      </Show>
    </div>
  );
}
