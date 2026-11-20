//! Selective disclosure of a committed, salted 52-card deal.

use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

type Hash = [u8; 32];
pub const VERSION: &str = "ms-poker-deck-v1";
const WIDTH: usize = 64;

fn digest(parts: &[&[u8]]) -> Hash {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update(part);
    }
    hash.finalize().into()
}

pub fn hex(bytes: &Hash) -> String {
    use std::fmt::Write;
    let mut value = String::with_capacity(64);
    for byte in bytes {
        write!(value, "{byte:02x}").unwrap();
    }
    value
}

pub struct DeckCommitment {
    cards: Vec<String>,
    salts: [Hash; WIDTH],
    nodes: [Hash; WIDTH * 2],
}

#[derive(Serialize)]
pub struct Opening {
    pub position: usize,
    pub card: String,
    pub salt: String,
    pub siblings: Vec<String>,
}

impl DeckCommitment {
    /// Commits to a seeded deck in draw order. Keep the seed private.
    pub fn new(id: Uuid, seed: Hash, button: u8, seats: u8) -> Self {
        let cards = crate::deal_order(seed);
        let salts = std::array::from_fn(|i| {
            digest(&[b"ms-poker-salt-v1", &seed, id.as_bytes(), &[i as u8]])
        });
        let mut nodes = [[0; 32]; WIDTH * 2];
        for i in 0..WIDTH {
            let card = cards.get(i).map_or("--", String::as_str);
            nodes[WIDTH + i] = digest(&[
                &[0],
                VERSION.as_bytes(),
                id.as_bytes(),
                &[button, seats, i as u8],
                card.as_bytes(),
                &salts[i],
            ]);
        }
        for i in (1..WIDTH).rev() {
            nodes[i] = digest(&[&[1], &nodes[2 * i], &nodes[2 * i + 1]]);
        }
        Self {
            cards,
            salts,
            nodes,
        }
    }

    pub fn root(&self) -> String {
        hex(&self.nodes[1])
    }

    /// Opens a zero-based draw position. Panics for positions outside the 52 cards.
    pub fn open(&self, position: usize) -> Opening {
        let mut node = WIDTH + position;
        let mut siblings = Vec::with_capacity(6);
        while node > 1 {
            siblings.push(hex(&self.nodes[node ^ 1]));
            node /= 2;
        }
        Opening {
            position,
            card: self.cards[position].clone(),
            salt: hex(&self.salts[position]),
            siblings,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commitment_matches_dealer_for_every_seat_count_and_button() {
        for seats in 2..=6 {
            for button in 0..seats {
                let id = Uuid::from_u128(1);
                let seed = [7; 32];
                let tree = DeckCommitment::new(id, seed, button, seats);
                let hand = crate::six_max::Hand::with_stacks(
                    seed,
                    button.into(),
                    &vec![1000; seats as usize],
                )
                .unwrap();
                for seat in 0..seats {
                    let position = (seat + seats - button - 1) % seats;
                    assert_eq!(
                        hand.hole(seat.into()),
                        [
                            tree.cards[position as usize].clone(),
                            tree.cards[(position + seats) as usize].clone()
                        ]
                    );
                }
                let position = 2 * seats + 7;
                let opened = tree.open(position as usize);
                assert_eq!(opened.siblings.len(), 6);
                let mut hash = digest(&[
                    &[0],
                    VERSION.as_bytes(),
                    id.as_bytes(),
                    &[button, seats, position],
                    opened.card.as_bytes(),
                    &tree.salts[position as usize],
                ]);
                let mut index = WIDTH + position as usize;
                while index > 1 {
                    let sibling = tree.nodes[index ^ 1];
                    hash = if index % 2 == 0 {
                        digest(&[&[1], &hash, &sibling])
                    } else {
                        digest(&[&[1], &sibling, &hash])
                    };
                    index /= 2;
                }
                assert_eq!(hash, tree.nodes[1]);
            }
        }
    }
}
