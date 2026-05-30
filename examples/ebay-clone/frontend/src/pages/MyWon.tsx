import { createResource, For, Show } from 'solid-js';
import { http } from '../api/http';
import { API_BASE } from '../api/auth';

type Won = { listing_id: number; winner: string; amount_cents: number };

async function fetchWon(): Promise<Won[]> {
  const r = await http<{ won: Won[] }>('GET', `${API_BASE}/api/v1/me/won`);
  return r.won ?? [];
}

export default function MyWon() {
  const [won] = createResource(fetchWon);
  return (
    <div class="p-6 max-w-2xl mx-auto">
      <h1 class="text-2xl font-bold mb-4">My won items</h1>
      <Show when={!won.loading} fallback={<p>Loading…</p>}>
        <For each={won()} fallback={<p class="text-gray-500">No won items yet.</p>}>
          {(w) => (
            <div class="border rounded p-3 mb-2">
              <div class="font-semibold">Listing #{w.listing_id}</div>
              <div class="text-sm text-gray-600">
                won by {w.winner} for ${(w.amount_cents / 100).toFixed(2)}
              </div>
            </div>
          )}
        </For>
      </Show>
    </div>
  );
}
