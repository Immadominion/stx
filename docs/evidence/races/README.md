# The Race: naive baseline vs stx (aggregate)

Six head-to-head races on mainnet under a contested tip floor. Each race fires the same transfer against the same live floor snapshot, then runs two strategies back to back (sequentially, so neither starves the other's per-region rate budget):

- **naive**: a fixed median (p50) tip, a single global endpoint, blind retry at the same tip. What an unspecialized submit loop does.
- **stx**: an EMA-smoothed, congestion-aware tip, fan-out to all 7 Jito regions, diagnose-and-escalate retries, bounded by a spend ceiling.

Both lanes are confirmed identically (validator stream cross-checked with RPC), so the only variable is the strategy. One JSON trace per lane per race is in this folder.

| Race | Naive | stx | stx landed slot |
|---|---|---|---|
| 1 | never landed (held 4,807) | landed in 4 (tip 100,000) | [428,109,591](https://solscan.io/tx/4FzLEiX4Uj2gKNzXm21G3DaSxqd9acNRe41wKRXgCmKQYkefXs7NrbuK8cR57pDQyuDxCHnzX33pDw2mfS3BNqNd) |
| 2 | never landed (held 1,077) | landed in 3 (tip 100,000) | [428,109,945](https://solscan.io/tx/2VShE8ahtn6RBemqQjGA5PbqoBNt3wWBiieAmmgrxj2teLp8nqCBfD7igjwrtvqsSUzgiSF4Dsfdhe4CKHTSs5NH) |
| 3 | never landed (held 10,957) | landed in 3 (tip 502,142) | [428,110,258](https://solscan.io/tx/55zyMkZWJKJ342S9mxNwjMbwczPKPFHYH14wWPQ5FamzpYvdRfb2YjLmr9oyczhbVvFihWBNy6SWmwn6fMA8Q73N) |
| 4 | never landed (held 1,976) | landed in 3 (tip 100,000) | [428,110,618](https://solscan.io/tx/4GH6aW1Aqj6yYKhR2pdVYpiBEMx3oVWa8v1STqUEsGZ3kcjbTKCzrxG9LwkarHZWxY69DJ9w3iDFyzZuj6xhDJ3p) |
| 5 | never landed (held 1,910) | landed in 4 (tip 7,401,001) | [428,111,031](https://solscan.io/tx/2Gaw3UPB12pWqpQdCqS1eHobLWXp6KkWZNyYZvueRFr4U3WYFPGiwQDPrHE7FWPRd9AP2r47QDM2yMcZZUTkqbK3) |
| 6 | never landed (held 2,504) | landed in 4 (tip 133,893) | [428,111,460](https://solscan.io/tx/VQ5snzHmY8qRDEPkXBqG8gcVkgwe2FWwcP9cUbLA1FvJ7hA8rYHLcCx9KrmzEdXjak4UqXmwXJ5VHdBDdNyzuYi) |

## Result: naive landed 0/6, stx landed 6/6

Every stx landing is finalized and explorer-verifiable (click a slot). The naive lane never won the auction: a fixed tip cannot keep up with a rising contested floor, while stx escalates to where landings are actually happening. Under a calm floor both strategies land; the gap is what the engineering buys you exactly when it is hard.
