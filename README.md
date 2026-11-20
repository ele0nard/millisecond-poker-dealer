# Millisecond Poker dealer

This is the Rust dealer library used by Millisecond Poker. It takes a seed, button position, starting stacks, and player decisions, then reproduces the deal and result. The venue pins this library to a Git revision. The [dealer guide](https://poker-docs.clearresolve.io/guide/dealer) describes the live flow and its verification limits.

| Code | What to inspect |
| --- | --- |
| [Six-seat engine](src/six_max.rs) | Deal order, blinds, action legality, check-or-fold fallback, all-ins, side pots, and odd chips |
| [Deck and heads-up engine](src/lib.rs) | ChaCha20 shuffle, hand ranking, and the separate heads-up game |
| [Table selection](src/seating.rs) | Oldest eligible owner, recent opponent penalties, known ownership groups, and position balancing |
| [Deck commitment](src/commitment.rs) | Salted SHA-256 Merkle root and proofs for cards at specific positions |
| [Public card verifier](proof/verify-shuffle.mjs) | The browser verifier’s card-checking logic, runnable from Node.js |

## Run the dealer tests

Use Rust 1.94 or newer.

```sh
cargo test --locked
```

The tests include a known seeded deck, check-or-fold timeouts, a short all-in, a tied side pot with an odd chip, card visibility, and chip conservation. The seating tests use synthetic owners and show how previous tables affect pairing and positions.

## Verify a public hand

The [sample](proof/sample) comes from a completed, timestamped USD Play Money hand. It contains the public API response, the manifest containing the hand ID and committed root, and the signed FreeTSA receipt. Run the card check with Node.js 22 or newer.

```sh
node proof/check-hand.mjs proof/sample/hand.json proof/sample/manifest.txt
```

Download the [FreeTSA CA certificate](https://freetsa.org/files/cacert.pem) as `cacert.pem` to authenticate the manifest with OpenSSL.

```sh
openssl ts -verify -data proof/sample/manifest.txt -in proof/sample/receipt.tsr -CAfile cacert.pem
openssl ts -reply -in proof/sample/receipt.tsr -text
```

To repeat the check with another hand, get the public JSON from `https://poker.clearresolve.io/api/six/shuffle/HAND_ID`, then download the manifest and receipt from the [hand verifier](https://poker.clearresolve.io/fairness#shuffle).

A successful card check ties each revealed card and its deck position to the manifest root. The signed receipt establishes when that manifest existed. Seed selection, live card secrecy, queue eligibility, and the time strategies received their first cards still depend on the running venue. The library tests show what the engine does given its inputs. Public proofs cover the cards that were revealed in a specific hand.

## Copyright

Copyright © 2026 Elvir Leonard. See [LICENSE](LICENSE) for inspection and verification permissions.
