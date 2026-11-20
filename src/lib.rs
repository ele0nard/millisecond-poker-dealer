use rand::{SeedableRng, seq::SliceRandom};
use rand_chacha::ChaCha20Rng;
use serde::{Deserialize, Serialize};

pub mod commitment;
pub mod seating;
pub mod six_max;

pub const STACK: u32 = 1_000;
pub const SMALL: u32 = 5;
pub const BIG: u32 = 10;
pub const MAX_HEADS_UP_STACK: u32 = 1_000_000;
pub const HANDS: u32 = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Card(u8);

impl std::fmt::Display for Card {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let rank = b"23456789TJQKA"[usize::from(self.0 % 13)] as char;
        let suit = b"cdhs"[usize::from(self.0 / 13)] as char;
        write!(f, "{rank}{suit}")
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Move {
    Check,
    Fold,
    Call,
    Raise { to: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Legal {
    pub check: bool,
    pub fold: bool,
    pub call: Option<u32>,
    pub raise: Option<[u32; 2]>,
}

impl Legal {
    pub fn allows(&self, action: Move) -> bool {
        match action {
            Move::Check => self.check,
            Move::Fold => self.fold,
            Move::Call => self.call.is_some(),
            Move::Raise { to } => self
                .raise
                .is_some_and(|[min, max]| (min..=max).contains(&to)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Result {
    pub delta: [i32; 2],
    pub showdown: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Street {
    Preflop,
    Flop,
    Turn,
    River,
}

impl std::fmt::Display for Street {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Preflop => "preflop",
            Self::Flop => "flop",
            Self::Turn => "turn",
            Self::River => "river",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub button: usize,
    pub actor: Option<usize>,
    pub street: Street,
    pub board: Vec<String>,
    pub stacks: [u32; 2],
    pub bets: [u32; 2],
    pub pot: u32,
}

pub struct Hand {
    button: usize,
    deck: Vec<Card>,
    hole: [[Card; 2]; 2],
    board: Vec<Card>,
    starting_stacks: [u32; 2],
    stacks: [u32; 2],
    street_bets: [u32; 2],
    committed: [u32; 2],
    acted: [bool; 2],
    last_raise: u32,
    big_blind: u32,
    actor: usize,
    result: Option<Result>,
}

impl Hand {
    pub fn new(seed: [u8; 32], button: usize) -> Self {
        Self::with_stacks(seed, button, [STACK; 2])
    }

    pub fn with_stacks(seed: [u8; 32], button: usize, starting_stacks: [u32; 2]) -> Self {
        Self::with_blinds(seed, button, starting_stacks, SMALL, BIG)
    }

    pub fn with_blinds(
        seed: [u8; 32],
        button: usize,
        starting_stacks: [u32; 2],
        small_blind: u32,
        big_blind: u32,
    ) -> Self {
        Self::deal(
            seed,
            button,
            starting_stacks,
            small_blind,
            big_blind,
            button ^ 1,
        )
    }

    /// Replay recordings written by the original button-first heads-up dealer.
    pub fn replay_button_first(
        seed: [u8; 32],
        button: usize,
        stacks: [u32; 2],
        small_blind: u32,
        big_blind: u32,
    ) -> Self {
        Self::deal(seed, button, stacks, small_blind, big_blind, button)
    }

    fn deal(
        seed: [u8; 32],
        button: usize,
        starting_stacks: [u32; 2],
        small_blind: u32,
        big_blind: u32,
        first: usize,
    ) -> Self {
        assert!(button < 2);
        assert!((1..=big_blind).contains(&small_blind) && big_blind <= MAX_HEADS_UP_STACK);
        assert!(
            starting_stacks
                .into_iter()
                .all(|stack| (1..=MAX_HEADS_UP_STACK).contains(&stack))
        );
        let mut deck = shuffled_deck(seed);
        let mut hole = [[Card(0); 2]; 2];
        for round in 0..2 {
            for seat in [first, first ^ 1] {
                hole[seat][round] = deck.pop().unwrap();
            }
        }
        let mut hand = Self {
            button,
            deck,
            hole,
            board: Vec::new(),
            starting_stacks,
            stacks: starting_stacks,
            street_bets: [0; 2],
            committed: [0; 2],
            acted: [false; 2],
            last_raise: big_blind,
            big_blind,
            actor: button,
            result: None,
        };
        hand.pay(button, small_blind.min(starting_stacks[button]));
        hand.pay(button ^ 1, big_blind.min(starting_stacks[button ^ 1]));
        // An all-in blind only leaves a decision if the small blind can call a larger bet.
        if hand.stacks[button] == 0
            || (hand.stacks[button ^ 1] == 0
                && hand.street_bets[button] >= hand.street_bets[button ^ 1])
        {
            hand.showdown();
        }
        hand
    }

    pub fn actor(&self) -> Option<usize> {
        self.result.is_none().then_some(self.actor)
    }

    pub fn hole(&self, seat: usize) -> [String; 2] {
        self.hole[seat].map(|card| card.to_string())
    }

    pub fn result(&self) -> Option<&Result> {
        self.result.as_ref()
    }

    /// Completed contested pot; unmatched contributions have been returned.
    pub fn settled_pot(&self) -> Option<u32> {
        self.result
            .as_ref()
            .map(|_| self.committed[0].min(self.committed[1]) * 2)
    }

    pub fn view(&self) -> View {
        View {
            button: self.button,
            actor: self.actor(),
            street: match self.board.len() {
                0 => Street::Preflop,
                3 => Street::Flop,
                4 => Street::Turn,
                _ => Street::River,
            },
            board: self.board.iter().map(ToString::to_string).collect(),
            stacks: self.stacks,
            bets: self.street_bets,
            pot: if self.result.is_some() {
                0
            } else {
                self.committed.iter().sum()
            },
        }
    }

    pub fn legal(&self) -> Option<Legal> {
        let seat = self.actor()?;
        let other = seat ^ 1;
        let high = self.street_bets[other];
        let due = high.saturating_sub(self.street_bets[seat]);
        let max = self.street_bets[seat] + self.stacks[seat];
        let raise = (!self.acted[seat] && self.stacks[other] > 0 && max > high)
            .then_some([(high + self.last_raise).min(max), max]);
        Some(Legal {
            check: due == 0,
            fold: due > 0,
            call: (due > 0).then_some(due.min(self.stacks[seat])),
            raise,
        })
    }

    pub fn allows(&self, action: Move) -> bool {
        self.legal().is_some_and(|legal| legal.allows(action))
    }

    pub fn timeout(&self) -> Option<Move> {
        self.legal()
            .map(|legal| if legal.check { Move::Check } else { Move::Fold })
    }

    pub fn play(&mut self, action: Move) -> bool {
        if !self.allows(action) {
            return false;
        }
        let seat = self.actor;
        let other = seat ^ 1;
        match action {
            Move::Fold => {
                self.finish(Some(other));
                return true;
            }
            Move::Check => {}
            Move::Call => self.pay(seat, self.legal().unwrap().call.unwrap()),
            Move::Raise { to } => {
                let increment = to - self.street_bets[other];
                if increment >= self.last_raise {
                    self.last_raise = increment;
                    self.acted[other] = false;
                }
                self.pay(seat, to - self.street_bets[seat]);
            }
        }
        self.acted[seat] = true;
        let matched = self.street_bets[0] == self.street_bets[1];
        let all_in = self.stacks.contains(&0);
        if (matched && self.acted == [true; 2])
            || (all_in && (matched || self.street_bets[seat] < self.street_bets[other]))
        {
            if all_in || self.board.len() == 5 {
                self.showdown();
            } else {
                self.deal_street();
                self.street_bets = [0; 2];
                self.acted = [false; 2];
                self.last_raise = self.big_blind;
                self.actor = self.button ^ 1;
            }
        } else {
            self.actor = other;
        }
        true
    }

    fn pay(&mut self, seat: usize, amount: u32) {
        self.stacks[seat] -= amount;
        self.street_bets[seat] += amount;
        self.committed[seat] += amount;
    }

    fn deal_street(&mut self) {
        self.deck.pop(); // burn
        let count = if self.board.is_empty() { 3 } else { 1 };
        self.board
            .extend((0..count).map(|_| self.deck.pop().unwrap()));
    }

    fn showdown(&mut self) {
        while self.board.len() < 5 {
            self.deal_street();
        }
        self.finish(None);
    }

    fn finish(&mut self, folded_winner: Option<usize>) {
        let matched = self.committed[0].min(self.committed[1]);
        for seat in 0..2 {
            self.stacks[seat] += self.committed[seat] - matched;
        }
        let winner = folded_winner.or_else(|| {
            let ranks = self.hole.map(|hole| best(hole, &self.board));
            match ranks[0].cmp(&ranks[1]) {
                std::cmp::Ordering::Greater => Some(0),
                std::cmp::Ordering::Less => Some(1),
                std::cmp::Ordering::Equal => None,
            }
        });
        if let Some(seat) = winner {
            self.stacks[seat] += matched * 2;
        } else {
            for stack in &mut self.stacks {
                *stack += matched;
            }
        }
        self.street_bets = [0; 2];
        self.result = Some(Result {
            delta: std::array::from_fn(|seat| {
                self.stacks[seat] as i32 - self.starting_stacks[seat] as i32
            }),
            showdown: folded_winner.is_none(),
        });
    }
}

/// Cards in the order the dealer would draw them, including burn positions.
pub fn deal_order(seed: [u8; 32]) -> Vec<String> {
    shuffled_deck(seed)
        .iter()
        .rev()
        .map(ToString::to_string)
        .collect()
}

fn shuffled_deck(seed: [u8; 32]) -> Vec<Card> {
    let mut deck: Vec<_> = (0..52).map(Card).collect();
    deck.shuffle(&mut ChaCha20Rng::from_seed(seed));
    deck
}

/// Rank a revealed Hold'em hand. Larger ranks win. Invalid or duplicate cards
/// within the seven-card hand return `None`.
pub fn showdown_rank(hole: [&str; 2], board: [&str; 5]) -> Option<[u8; 6]> {
    let mut cards = [Card(0); 7];
    let mut seen = 0u64;
    for (out, code) in cards.iter_mut().zip(hole.into_iter().chain(board)) {
        let &[rank, suit] = code.as_bytes() else {
            return None;
        };
        let rank = b"23456789TJQKA".iter().position(|&r| r == rank)?;
        let suit = b"cdhs".iter().position(|&s| s == suit)?;
        let index = 13 * suit + rank;
        if seen & (1 << index) != 0 {
            return None;
        }
        seen |= 1 << index;
        *out = Card(index as u8);
    }
    Some(best([cards[0], cards[1]], &cards[2..]))
}

fn best(hole: [Card; 2], board: &[Card]) -> [u8; 6] {
    let cards = [hole.as_slice(), board].concat();
    let mut rank = [0; 6];
    for i in 0..7 {
        for j in i + 1..7 {
            let mut five = [Card(0); 5];
            for (out, card) in five.iter_mut().zip((0..7).filter(|n| *n != i && *n != j)) {
                *out = cards[card];
            }
            rank = rank.max(rank_five(five));
        }
    }
    rank
}

fn rank_five(cards: [Card; 5]) -> [u8; 6] {
    let flush = cards.iter().all(|card| card.0 / 13 == cards[0].0 / 13);
    let mut counts = [0; 15];
    for card in cards {
        counts[usize::from(card.0 % 13 + 2)] += 1;
    }
    let mut groups: Vec<_> = (2..=14u8)
        .filter_map(|rank| {
            let count = counts[usize::from(rank)];
            (count > 0).then_some((count, rank))
        })
        .collect();
    groups.sort_unstable_by(|a, b| b.cmp(a));
    let straight = if groups.len() == 5 {
        let high = groups[0].1;
        if high - groups[4].1 == 4 {
            high
        } else if high == 14 && groups[1].1 == 5 {
            5
        } else {
            0
        }
    } else {
        0
    };
    let category = match (flush, straight, groups.as_slice()) {
        (true, 1.., _) => return [8, straight, 0, 0, 0, 0],
        (_, _, [(4, _), ..]) => 7,
        (_, _, [(3, _), (2, _)]) => 6,
        (true, _, _) => 5,
        (_, 1.., _) => return [4, straight, 0, 0, 0, 0],
        (_, _, [(3, _), ..]) => 3,
        (_, _, [(2, _), (2, _), ..]) => 2,
        (_, _, [(2, _), ..]) => 1,
        _ => 0,
    };
    let mut rank = [0; 6];
    rank[0] = category;
    for (out, (_, value)) in rank[1..].iter_mut().zip(groups) {
        *out = value;
    }
    rank
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heads_up_known_deck_deals_last_card_to_button_in_both_engines() {
        // Independently reproduced from a zero seed: 5s, 4c, 3h, 9d, burn Ac.
        for button in 0..2 {
            let heads = Hand::new([0; 32], button);
            let pool = six_max::Hand::with_stacks([0; 32], button, &[STACK; 2]).unwrap();
            for seat in 0..2 {
                let expected = if seat == button {
                    ["4c", "9d"]
                } else {
                    ["5s", "3h"]
                };
                assert_eq!(heads.hole(seat), expected);
                assert_eq!(pool.hole(seat), expected);
            }
            let legacy = Hand::replay_button_first([0; 32], button, [STACK; 2], SMALL, BIG);
            assert_eq!(legacy.hole(button), ["5s", "3h"]);
        }
    }

    #[test]
    fn blind_option_and_postflop_order() {
        let mut hand = Hand::new([7; 32], 1);
        assert_eq!(hand.actor(), Some(1));
        assert_eq!(hand.view().bets, [10, 5]);
        assert_eq!(hand.legal().unwrap().raise, Some([20, 1000]));
        assert!(hand.play(Move::Call));
        assert_eq!(hand.actor(), Some(0));
        assert!(hand.play(Move::Check));
        assert_eq!(hand.view().street, Street::Flop);
        assert_eq!(hand.actor(), Some(0));
    }

    #[test]
    fn minimum_and_short_all_in() {
        let mut hand = Hand::new([2; 32], 0);
        assert!(!hand.play(Move::Raise { to: 19 }));
        assert!(hand.play(Move::Raise { to: 520 }));
        assert_eq!(hand.legal().unwrap().raise, Some([1000, 1000]));
        assert!(hand.play(Move::Raise { to: 1000 }));
        assert_eq!(hand.legal().unwrap().raise, None);
        assert_eq!(hand.legal().unwrap().call, Some(480));
        assert!(hand.play(Move::Call));
        assert!(hand.result().unwrap().showdown);
        assert_eq!(hand.view().board.len(), 5);
        assert_eq!(hand.result().unwrap().delta.iter().sum::<i32>(), 0);
    }

    #[test]
    fn custom_blinds_set_the_minimum_raise_on_each_street() {
        let mut hand = Hand::with_blinds([5; 32], 0, [250_000, 300_000], 1250, 2500);
        assert_eq!(hand.view().bets, [1250, 2500]);
        assert_eq!(hand.legal().unwrap().raise, Some([5000, 250_000]));
        assert!(hand.play(Move::Call));
        assert!(hand.play(Move::Check));
        assert_eq!(hand.legal().unwrap().raise, Some([2500, 297_500]));
    }

    #[test]
    fn short_stacks_can_play_below_either_blind() {
        for (stacks, call) in [
            ([3, 1000], None),
            ([1000, 3], None),
            ([1000, 7], Some(2)),
            ([7, 1000], Some(2)),
        ] {
            let mut hand = Hand::with_stacks([23; 32], 0, stacks);
            if let Some(call) = call {
                assert_eq!(hand.legal().unwrap().call, Some(call));
                assert_eq!(hand.legal().unwrap().raise, None);
                assert!(hand.play(Move::Call));
            }
            let result = hand.result().unwrap();
            assert!(result.showdown);
            assert_eq!(hand.settled_pot(), Some(2 * stacks[0].min(stacks[1])));
            assert_eq!(hand.view().stacks.iter().sum::<u32>(), stacks.iter().sum());
            for seat in 0..2 {
                assert_eq!(
                    hand.view().stacks[seat] as i32,
                    stacks[seat] as i32 + result.delta[seat]
                );
            }
        }
    }

    #[test]
    fn full_reraise_reopens_action_with_the_new_increment() {
        let mut hand = Hand::new([3; 32], 0);
        assert!(hand.play(Move::Raise { to: 40 }));
        assert_eq!(hand.legal().unwrap().raise, Some([70, 1000]));
        assert!(hand.play(Move::Raise { to: 100 }));
        assert_eq!(hand.actor(), Some(0));
        assert_eq!(hand.legal().unwrap().raise, Some([160, 1000]));
        assert!(!hand.play(Move::Raise { to: 159 }));
        assert!(hand.play(Move::Raise { to: 160 }));
    }

    #[test]
    fn uncalled_wager_returns_on_fold() {
        let mut hand = Hand::new([0; 32], 0);
        assert!(hand.play(Move::Raise { to: 1000 }));
        assert_eq!(hand.timeout(), Some(Move::Fold));
        assert!(hand.play(Move::Fold));
        assert_eq!(hand.result().unwrap().delta, [10, -10]);
        assert!(!hand.result().unwrap().showdown);
    }

    #[test]
    fn royal_board_splits_and_wheel_ranks_below_six_high() {
        let mut hand = Hand::new([0; 32], 0);
        hand.board = vec![Card(8), Card(9), Card(10), Card(11), Card(12)];
        hand.hole = [[Card(1), Card(13)], [Card(26), Card(27)]];
        hand.pay(0, 5);
        hand.finish(None);
        assert_eq!(hand.result().unwrap().delta, [0, 0]);
        let wheel = rank_five([Card(12), Card(0), Card(14), Card(2), Card(3)]);
        let six = rank_five([Card(0), Card(14), Card(2), Card(3), Card(4)]);
        assert_eq!(wheel[0..2], [4, 5]);
        assert!(six > wheel);
    }
}
