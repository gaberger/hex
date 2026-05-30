import { createSignal, Show } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { http } from '../api/http';
import { API_BASE, getToken } from '../api/auth';

export default function PostListing() {
  const [title, setTitle] = createSignal('');
  const [description, setDescription] = createSignal('');
  const [price, setPrice] = createSignal('10.00');
  const [durationSecs, setDurationSecs] = createSignal('3600');
  const [error, setError] = createSignal('');
  const navigate = useNavigate();

  async function submit(e: Event) {
    e.preventDefault();
    setError('');
    if (!getToken()) {
      setError('Please register or login first.');
      return;
    }
    try {
      await http('POST', `${API_BASE}/api/v1/listings`, {
        title: title(),
        description: description(),
        starting_price_cents: Math.round(parseFloat(price()) * 100),
        duration_secs: parseInt(durationSecs(), 10),
      });
      navigate('/');
    } catch (err: any) {
      setError(String(err?.message ?? err));
    }
  }

  return (
    <form onSubmit={submit} class="p-6 max-w-sm mx-auto space-y-3">
      <h1 class="text-2xl font-bold">Post a listing</h1>
      <input class="border p-2 w-full rounded" placeholder="title"
        value={title()} onInput={(e) => setTitle(e.currentTarget.value)} />
      <input class="border p-2 w-full rounded" placeholder="description"
        value={description()} onInput={(e) => setDescription(e.currentTarget.value)} />
      <label class="block text-sm">Starting price ($)
        <input class="border p-2 w-full rounded" type="number" step="0.01"
          value={price()} onInput={(e) => setPrice(e.currentTarget.value)} />
      </label>
      <label class="block text-sm">Auction duration (seconds)
        <input class="border p-2 w-full rounded" type="number"
          value={durationSecs()} onInput={(e) => setDurationSecs(e.currentTarget.value)} />
      </label>
      <button class="bg-blue-600 text-white px-4 py-2 rounded" type="submit">Post</button>
      <Show when={error()}><p class="text-red-600">{error()}</p></Show>
    </form>
  );
}
