//! Table selection and position balancing for an eligible shortlist.

use crate::six_max::SEATS;
use rand::{Rng, seq::SliceRandom};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub trait Candidate {
    /// Identifies the owner for matching against recent hand history.
    fn owner_id(&self) -> Uuid;
    /// Returns the known ownership group, or `None` to use the owner ID for grouping.
    fn known_group(&self) -> Option<Uuid>;
}

/// Selects a table from an oldest-first shortlist and returns the seats and button index.
/// Each history row holds up to six distinct owners, starting with the button.
///
/// # Panics
///
/// Panics if the shortlist is empty.
pub fn choose<C: Candidate + Clone>(
    mut entries: Vec<C>,
    history: &[Vec<Uuid>],
    rng: &mut (impl Rng + ?Sized),
) -> (Vec<C>, usize) {
    let count = entries
        .iter()
        .map(group_key)
        .collect::<HashSet<_>>()
        .len()
        .min(SEATS);
    let candidates: HashSet<_> = entries.iter().map(|entry| entry.owner_id()).collect();
    let mut pairs = HashMap::<(Uuid, Uuid), u32>::new();
    let mut positions = HashMap::<Uuid, [i32; SEATS]>::new();
    for hand in history {
        for (position, &owner) in hand.iter().enumerate() {
            if !candidates.contains(&owner) {
                continue;
            }
            if hand.len() == count {
                positions.entry(owner).or_default()[position] += 1;
            }
            for &opponent in hand {
                if owner != opponent && candidates.contains(&opponent) {
                    *pairs.entry((owner, opponent)).or_default() += 1;
                }
            }
        }
    }
    // Every table includes the oldest eligible owner. A bounded shortlist is
    // ordered by wait time before this random tie-break and repeat penalty.
    let mut group = vec![entries.remove(0)];
    let mut selected = HashSet::from([group_key(&group[0])]);
    entries.shuffle(rng);
    while group.len() < count {
        let (index, _) = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| !selected.contains(&group_key(*entry)))
            .min_by_key(|(_, candidate)| {
                group
                    .iter()
                    .map(|entry| {
                        pairs
                            .get(&(candidate.owner_id(), entry.owner_id()))
                            .copied()
                            .unwrap_or(0)
                    })
                    .sum::<u32>()
            })
            .unwrap();
        let entry = entries.remove(index);
        selected.insert(group_key(&entry));
        group.push(entry);
    }
    group.shuffle(rng);
    let button = rng.random_range(0..count);
    /// Finds the lowest-cost seat assignment, retaining the first of any tied solutions.
    fn best_assignment(
        costs: &[[i32; SEATS]; SEATS],
        count: usize,
        level: usize,
        used: u8,
        cost: i32,
        order: &mut [usize; SEATS],
        best: &mut (i32, [usize; SEATS]),
    ) {
        if level == count {
            if cost < best.0 {
                *best = (cost, *order);
            }
            return;
        }
        for candidate in 0..count {
            let bit = 1 << candidate;
            if used & bit == 0 {
                order[level] = candidate;
                best_assignment(
                    costs,
                    count,
                    level + 1,
                    used | bit,
                    cost + costs[candidate][level],
                    order,
                    best,
                );
            }
        }
    }
    // Favor positions each owner has played less often than their own average.
    let costs = std::array::from_fn(|candidate| {
        let Some(entry) = group.get(candidate) else {
            return [0; SEATS];
        };
        let counts = positions
            .get(&entry.owner_id())
            .copied()
            .unwrap_or_default();
        let total = counts.iter().sum::<i32>();
        counts.map(|count| count * group.len() as i32 - total)
    });
    let mut best = (i32::MAX, [0; SEATS]);
    best_assignment(&costs, count, 0, 0, 0, &mut [0; SEATS], &mut best);
    (
        (0..count)
            .map(|seat| group[best.1[(seat + count - button) % count]].clone())
            .collect(),
        button,
    )
}

/// Separates ownership-group IDs from individual owner IDs, even when their UUIDs match.
fn group_key(entry: &impl Candidate) -> (bool, Uuid) {
    (
        entry.known_group().is_some(),
        entry.known_group().unwrap_or(entry.owner_id()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::StdRng};

    #[derive(Clone)]
    struct Entry {
        owner: Uuid,
        group: Option<Uuid>,
    }

    impl Candidate for Entry {
        fn owner_id(&self) -> Uuid {
            self.owner
        }

        fn known_group(&self) -> Option<Uuid> {
            self.group
        }
    }

    /// Creates distinct synthetic owners in queue order.
    fn entries(count: u128) -> Vec<Entry> {
        (1..=count)
            .map(|id| Entry {
                owner: Uuid::from_u128(id),
                group: None,
            })
            .collect()
    }

    /// Projects a selected table into the button-first order used by hand history.
    fn normalize(seats: &[Entry], button: usize) -> Vec<Uuid> {
        (0..seats.len())
            .map(|position| seats[(position + button) % seats.len()].owner)
            .collect()
    }

    #[test]
    fn oldest_owner_is_served_and_repeated_opponents_are_avoided() {
        let mut random = StdRng::seed_from_u64(41);
        let cohort = entries(12);
        let old = cohort[..6]
            .iter()
            .map(|entry| entry.owner)
            .collect::<Vec<_>>();
        let (seats, _) = choose(
            cohort,
            &[old.clone(), old.clone(), old.clone()],
            &mut random,
        );
        assert!(seats.iter().any(|seat| seat.owner == old[0]));
        assert!(seats.iter().all(|seat| !old[1..].contains(&seat.owner)));
    }

    #[test]
    fn six_owners_each_get_every_position_over_six_deals() {
        let mut random = StdRng::seed_from_u64(42);
        let mut history = Vec::new();
        for _ in 0..6 {
            let (seats, button) = choose(entries(6), &history, &mut random);
            let hand = normalize(&seats, button);
            assert_eq!(hand.iter().collect::<HashSet<_>>().len(), 6);
            history.push(hand);
        }
        for position in 0..6 {
            assert_eq!(
                history
                    .iter()
                    .map(|hand| hand[position])
                    .collect::<HashSet<_>>()
                    .len(),
                6
            );
        }
    }

    #[test]
    fn verified_group_members_do_not_share_a_table() {
        let mut random = StdRng::seed_from_u64(43);
        let mut candidates = entries(7);
        candidates[0].group = Some(Uuid::from_u128(500));
        candidates[1].group = candidates[0].group;
        let excluded = candidates[1].owner;
        let (seats, _) = choose(candidates, &[], &mut random);
        assert!(seats.iter().all(|seat| seat.owner != excluded));
    }
}
