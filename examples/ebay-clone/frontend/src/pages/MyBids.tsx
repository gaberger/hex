import { createSignal, Show } from 'solid-js';
import { http } from '../api/http';
import { API_BASE, getToken } from '../api/auth';

// Places a bid on a listing by id — POST /api/v1/listings/:id/bids.
export default function MyBids() {
  const [listingId, setListingId] = createSignal('');
  const [amount, setAmount] = createSignal('15.00');
  const [msg, setMsg] = createSignal('');
  const [error, setError] = createSignal('');

  async function submit(e: Event) {
    e.preventDefault();
    setMsg(''); setError('');
    if (!getToken()) { setError('Please register or login first.'); return; }
    try {
      await http('POST', `${API_BASE}/api/v1/listings/${parseInt(listingId(), 10)}/bids`, {
        amount_cents: Math.round(parseFloat(amount()) * 100),
      });
      setMsg(`Bid placed on listing #${listingId()}.`);
    } catch (err: any) {
      setError(String(err?.message ?? err));
    }
  }

  return (
    <form onSubmit={submit} class="p-6 max-w-sm mx-auto space-y-3">
      <h1 class="text-2xl font-bold">Place a bid</h1>
      <input class="border p-2 w-full rounded" placeholder="listing id"
        value={listingId()} onInput={(e) => setListingId(e.currentTarget.value)} />
      <label class="block text-sm">Amount ($)
        <input class="border p-2 w-full rounded" type="number" step="0.01"
          value={amount()} onInput={(e) => setAmount(e.currentTarget.value)} />
      </label>
      <button class="bg-blue-600 text-white px-4 py-2 rounded" type="submit">Bid</button>
      <Show when={msg()}><p class="text-green-700">{msg()}</p></Show>
      <Show when={error()}><p class="text-red-600">{error()}</p></Show>
    </form>
  );
}
