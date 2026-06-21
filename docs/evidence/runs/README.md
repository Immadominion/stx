# Curated submission lifecycle log

Real mainnet runs of `stx submit`, exported straight from the append-only event store (one JSON per run in this folder). Landed runs climb the full commitment ladder; the `f0*` runs are single-attempt submissions under a deliberately short window that did not land, kept as honest failure cases. Every bundle is fanned out to all 7 Jito regions.

| Run | Outcome | Attempts | Tips (lamports) | Landed slot | Block leader | processed to confirmed | confirmed to finalized |
|---|---|---|---|---|---|---|---|
| run-01 | landed ([tx](https://solscan.io/tx/4LNE4ssiXwBQcszq3b5KErJCMGGNKiBvtoWGnR24X6bMemfW2CC1WmPvVTdMBBTiHb4QsxzUgkSBCYs1swr1ikBq)) | 2 | 8949, 100000 | [427829141](https://solscan.io/block/427829141) | `CoG8d9Fp...` | 0.645s | 12.359s |
| run-02 | landed ([tx](https://solscan.io/tx/2vCEieVHnb9iSBEADqBwABmZy6tfWpCbg1dDLE4HPSdsoCVCAyJKtSQpxD2nwycgmpKGPTGjAoRRFLHfMeZgjkbF)) | 3 | 2446, 10000, 126413 | [427829325](https://solscan.io/block/427829325) | `Av8EnYrP...` | 0.647s | 12.094s |
| run-03 | landed ([tx](https://solscan.io/tx/2zRkhMts1Xd3Lr2pN3rPvombMa3EdbhEJNTTeGfwgW8sstNB9nG4Lgt8NjNkQDjAR2FKcSZtX38zdRukRignb2vY)) | 3 | 2621, 11854, 100000 | [427829507](https://solscan.io/block/427829507) | `DUND26mE...` | 0.653s | 12.272s |
| run-04 | landed ([tx](https://solscan.io/tx/2v7uFmhZj5rRsXVB7KTMCUyugkf2uFJTK2iZPixX2ioyUwkfzubNmdRRRXnRDfYqupgo4wjU33XV9ZmJkbRi5rcD)) | 3 | 1864, 6000, 100000 | [427829693](https://solscan.io/block/427829693) | `RNXnAJV1...` | 0.656s | 12.179s |
| run-05 | landed ([tx](https://solscan.io/tx/2cJ6S36JAJ8BDkwcadGgMHUmnRMumTJN4fj6dM9FRwunPuV55c1wGTZWAztgFgsETdtUwsKKtAT6MfukbDb4Bm4i)) | 3 | 6000, 10000, 100000 | [427829878](https://solscan.io/block/427829878) | `CoG8d9Fp...` | 0.633s | 11.264s |
| run-06 | landed ([tx](https://solscan.io/tx/TFCie2cqXeDK9nF6gmVA1m2UXT1D89HNkUEjQwKSaKSdkp4vBtVjf6rYZSE6E2HBW7VqqeiG5DcCH8BA1d94euQ)) | 4 | 1001, 1985, 50500, 100000 | [427830136](https://solscan.io/block/427830136) | `Ex1AxFCi...` | 0.648s | 11.780s |
| run-07 | landed ([tx](https://solscan.io/tx/FkUbuWWqKxRkYmh7vXoYKLXiG11e75fUmABhN8WjoUr4sX53MRfhrW2sEa4dkvBGDFnDFpmNyEtDy9SUFNXzKGr)) | 3 | 2007, 16200, 408596 | [427830327](https://solscan.io/block/427830327) | `fishfish...` | 0.641s | 11.713s |
| run-f01 | not landed | 1 | 3519 | - | - | - | - |
| run-f02 | not landed | 1 | 3519 | - | - | - | - |
| run-f03 | not landed | 1 | 3861 | - | - | - | - |

**10 runs: 7 landed, 3 not landed.** The agent-steered run and the canonical fallback run live one level up as [`lifecycle-agent-run.json`](../lifecycle-agent-run.json) and [`lifecycle-fallback-run.json`](../lifecycle-fallback-run.json), with the agent's reasoning in [`agent-live-decisions.json`](../agent-live-decisions.json).
