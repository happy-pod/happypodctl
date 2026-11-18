# Recipe: the API gained a route

One change carries all four parts. A route that only some of them know about is
the top smell in `doctrine.md`.

## 0. It must be in the contract first

The route is in happy-pod/mcp `docs/API.md` and answered by its `src/demo.ts`.
If it is not, stop: that change comes first, in that repo.

## 1. `api-index.md`

Add the row: route, command, MCP tool, notes (errors, special behaviour).

## 2. `demo.rs`

Port the `demo.ts` handler.

- A route under `/pods/:id/...` goes in `pod_route`; add its shape to the
  `r.starts_with(...)` guard if it uses a new method, and its fixed word to
  `FIXED` (see `errors.md` §6).
- Anything else goes in the `match route.as_str()` in `handle`.
- Errors: `err(status, "CODE", "Message as demo.ts words it")`, with
  `.with_details(json!({...}))` when `demo.ts` sends details.
- A create that should honour Idempotency-Key returns through `created(st, v)`.
- A GET that changes state must also be saved: extend the condition in
  `Sim::send` (`errors.md` §7).
- New state fields: `#[serde(default)]`, so existing `demo.json` files load.

Add a unit test in `demo.rs` `mod tests` for any logic beyond a lookup.

## 3. The command

`recipes/command.md`.

## 4. The end-to-end test

`recipes/testing.md`. Cover the error the agent is most likely to hit, not only
the happy path.
