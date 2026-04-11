# Counter App

A simple counter app to test the steerable agent loop.

## Test Scenario

1. Start worker: `hex agent worker --role hex-coder`
2. Create a task in HexFlo
3. Send pause mid-execution: `hex agent pause <worker_id>`
4. Resume: `hex agent resume <worker_id>`
5. Verify output was generated

## Build

```bash
npm run dev
```

## Interact

- Click to increment counter
- Counter persists to localStorage