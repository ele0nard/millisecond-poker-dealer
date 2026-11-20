//! Deterministic two-to-six-player no-limit Hold'em with 5/10 blinds.

use crate::{BIG, Card, Legal, Move, SMALL, Street, best, shuffled_deck};
use serde::{Deserialize, Serialize};

pub const SEATS: usize = 6;
pub const MIN_SEATS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupError {
    InvalidButton,
    InvalidPlayerCount,
    EmptyStack,
    ChipTotalOverflow,
}

impl std::fmt::Display for SetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidButton => "button must identify an occupied seat",
            Self::InvalidPlayerCount => "a hand requires two to six seats",
            Self::EmptyStack => "every seat must start with chips",
            Self::ChipTotalOverflow => "the sum of starting stacks must fit in u32",
        })
    }
}

impl std::error::Error for SetupError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    pub button: usize,
    pub actor: Option<usize>,
    pub street: Street,
    pub board: Vec<String>,
    pub stacks: [u32; SEATS],
    pub bets: [u32; SEATS],
    pub contributions: [u32; SEATS],
    pub folded: [bool; SEATS],
    pub pot: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    pub seat: usize,
    pub hole: [String; 2],
    pub table: View,
    pub legal: Legal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PotAward {
    pub amount: u32,
    pub eligible: [bool; SEATS],
    /// Historical pot fee in whole chips. The earliest records omit this field.
    #[serde(default)]
    pub fee: u32,
    /// Net payouts, including odd chips, sum to the pot amount less the fee.
    pub payouts: [u32; SEATS],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    /// Main pot first, followed by side pots in ascending contribution order.
    pub pots: Vec<PotAward>,
    pub uncalled: [u32; SEATS],
    pub stacks: [u32; SEATS],
    pub delta: [i64; SEATS],
    pub showdown: bool,
    /// Only non-folded hands are revealed, and only at showdown.
    pub revealed: [Option<[String; 2]>; SEATS],
}

impl Outcome {
    pub fn total_fee(&self) -> u32 {
        self.pots.iter().map(|pot| pot.fee).sum()
    }
}

/// The dealer pays every chip. Legacy replay reproduces the historical single-winner fee.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PotPolicy {
    FullPayout,
    LegacySingleWinnerFee,
}

impl PotPolicy {
    pub fn recorded(rules: i16, has_fee_field: bool) -> Option<Self> {
        match (rules, has_fee_field) {
            (1, true) => Some(Self::LegacySingleWinnerFee),
            (1, false) | (2, _) => Some(Self::FullPayout),
            _ => None,
        }
    }
}

pub struct Hand {
    pot_policy: PotPolicy,
    button: usize,
    players: usize,
    deck: Vec<Card>,
    hole: [[Card; 2]; SEATS],
    board: Vec<Card>,
    starting: [u32; SEATS],
    stacks: [u32; SEATS],
    bets: [u32; SEATS],
    contributions: [u32; SEATS],
    folded: [bool; SEATS],
    /// Price at each seat's last voluntary action on this street.
    acted_at: [Option<u32>; SEATS],
    last_full_raise: u32,
    actor: Option<usize>,
    outcome: Option<Outcome>,
}

impl Hand {
    pub fn new(seed: [u8; 32], button: usize, stacks: [u32; SEATS]) -> Result<Self, SetupError> {
        Self::with_stacks(seed, button, &stacks)
    }

    pub fn with_stacks(seed: [u8; 32], button: usize, stacks: &[u32]) -> Result<Self, SetupError> {
        Self::with_pot_policy(seed, button, stacks, PotPolicy::FullPayout)
    }

    pub fn with_pot_policy(
        seed: [u8; 32],
        button: usize,
        stacks: &[u32],
        pot_policy: PotPolicy,
    ) -> Result<Self, SetupError> {
        let players = stacks.len();
        if !(MIN_SEATS..=SEATS).contains(&players) {
            return Err(SetupError::InvalidPlayerCount);
        }
        if button >= players {
            return Err(SetupError::InvalidButton);
        }
        if stacks.contains(&0) {
            return Err(SetupError::EmptyStack);
        }
        stacks
            .iter()
            .try_fold(0u32, |total, &stack| total.checked_add(stack))
            .ok_or(SetupError::ChipTotalOverflow)?;

        let mut deck = shuffled_deck(seed);
        let mut hole = [[Card(0); 2]; SEATS];
        for round in 0..2 {
            for seat in clockwise(button, players) {
                hole[seat][round] = deck.pop().unwrap();
            }
        }
        // Keep ABI 2 arrays six-wide. Empty seats have no cards or chips and
        // are marked folded so existing bots exclude them from play.
        let mut starting = [0; SEATS];
        starting[..players].copy_from_slice(stacks);
        let mut hand = Self {
            pot_policy,
            button,
            players,
            deck,
            hole,
            board: Vec::with_capacity(5),
            starting,
            stacks: starting,
            bets: [0; SEATS],
            contributions: [0; SEATS],
            folded: std::array::from_fn(|seat| seat >= players),
            acted_at: [None; SEATS],
            last_full_raise: BIG,
            actor: None,
            outcome: None,
        };
        let small = if players == 2 {
            button
        } else {
            (button + 1) % players
        };
        let big = (small + 1) % players;
        hand.pay(small, SMALL.min(stacks[small]));
        hand.pay(big, BIG.min(stacks[big]));
        hand.advance(big);
        Ok(hand)
    }

    pub fn player_count(&self) -> usize {
        self.players
    }

    /// Privileged inspection only; never include this in a player observation.
    pub fn hole(&self, seat: usize) -> [String; 2] {
        self.hole[seat].map(|card| card.to_string())
    }

    pub fn actor(&self) -> Option<usize> {
        self.actor
    }

    pub fn outcome(&self) -> Option<&Outcome> {
        self.outcome.as_ref()
    }

    pub fn view(&self) -> View {
        View {
            button: self.button,
            actor: self.actor,
            street: match self.board.len() {
                0 => Street::Preflop,
                3 => Street::Flop,
                4 => Street::Turn,
                _ => Street::River,
            },
            board: self.board.iter().map(ToString::to_string).collect(),
            stacks: self.stacks,
            bets: self.bets,
            contributions: self.contributions,
            folded: self.folded,
            pot: self.contributions.iter().sum(),
        }
    }

    /// Derive a decision input for the current actor without secret state
    /// from other seats. Callers must route it to that seat only.
    pub fn observation(&self) -> Option<Observation> {
        let seat = self.actor?;
        Some(Observation {
            seat,
            hole: self.hole[seat].map(|card| card.to_string()),
            table: self.view(),
            legal: self.legal()?,
        })
    }

    pub fn legal(&self) -> Option<Legal> {
        let seat = self.actor?;
        let can_bet = self.can_bet_count();
        let price = self.price(can_bet);
        let due = price.saturating_sub(self.bets[seat]);
        let max = self.bets[seat] + self.stacks[seat];
        let reopened = self.acted_at[seat]
            .is_none_or(|last| price.saturating_sub(last) >= self.last_full_raise);
        let raise = (can_bet > 1 && max > price && reopened)
            .then_some([price.saturating_add(self.last_full_raise).min(max), max]);
        Some(Legal {
            check: due == 0,
            fold: due > 0,
            call: (due > 0).then_some(due.min(self.stacks[seat])),
            raise,
        })
    }

    pub fn timeout(&self) -> Option<Move> {
        self.legal()
            .map(|legal| if legal.check { Move::Check } else { Move::Fold })
    }

    /// Reject out-of-turn and illegal moves without changing any hand state.
    pub fn play(&mut self, seat: usize, action: Move) -> bool {
        if self.actor != Some(seat) {
            return false;
        }
        let legal = self.legal().unwrap();
        if !legal.allows(action) {
            return false;
        }
        let price = self.price(self.can_bet_count());
        match action {
            Move::Fold => self.folded[seat] = true,
            Move::Check => {}
            Move::Call => self.pay(seat, legal.call.unwrap()),
            Move::Raise { to } => {
                self.last_full_raise = self.last_full_raise.max(to - price);
                self.pay(seat, to - self.bets[seat]);
            }
        }
        // A call records the price faced. An aggressive action records the
        // new price so successive short all-ins are measured from it.
        self.acted_at[seat] = Some(price.max(self.bets[seat]));
        self.advance(seat);
        true
    }

    fn pay(&mut self, seat: usize, amount: u32) {
        self.stacks[seat] -= amount;
        self.bets[seat] += amount;
        self.contributions[seat] += amount;
    }

    fn can_bet(&self, seat: usize) -> bool {
        !self.folded[seat] && self.stacks[seat] > 0
    }

    fn can_bet_count(&self) -> usize {
        (0..SEATS).filter(|&seat| self.can_bet(seat)).count()
    }

    fn price(&self, can_bet: usize) -> u32 {
        let posted = self.bets.into_iter().max().unwrap();
        // A short big blind does not reduce the preflop bring-in while
        // multiple players can still bet. A lone covering stack only
        // owes the amount actually wagered.
        if self.board.is_empty() && can_bet > 1 {
            posted.max(BIG)
        } else {
            posted
        }
    }

    fn advance(&mut self, mut after: usize) {
        loop {
            if self.folded.iter().filter(|&&folded| !folded).count() == 1 {
                self.finish(false);
                return;
            }
            let can_bet = self.can_bet_count();
            let price = self.price(can_bet);
            self.actor = clockwise(after, self.players).find(|&seat| {
                self.can_bet(seat)
                    && (self.bets[seat] < price || (can_bet > 1 && self.acted_at[seat].is_none()))
            });
            if self.actor.is_some() {
                return;
            }
            if self.board.len() == 5 {
                self.finish(true);
                return;
            }

            self.deck.pop().unwrap(); // burn
            let count = if self.board.is_empty() { 3 } else { 1 };
            self.board
                .extend((0..count).map(|_| self.deck.pop().unwrap()));
            self.bets = [0; SEATS];
            self.acted_at = [None; SEATS];
            self.last_full_raise = BIG;
            after = self.button;
        }
    }

    fn finish(&mut self, showdown: bool) {
        let ranks: [Option<[u8; 6]>; SEATS] = std::array::from_fn(|seat| {
            (!self.folded[seat] && showdown).then(|| best(self.hole[seat], &self.board))
        });
        let (layers, uncalled) = self.pot_layers();
        let pots = layers
            .into_iter()
            .map(|(amount, eligible)| {
                let best_rank = (0..SEATS)
                    .filter(|&seat| eligible[seat])
                    .map(|seat| ranks[seat])
                    .max()
                    .unwrap();
                let winners: Vec<_> = clockwise(self.button, self.players)
                    .filter(|&seat| eligible[seat] && ranks[seat] == best_rank)
                    .collect();
                let count = winners.len() as u32;
                let fee = match self.pot_policy {
                    PotPolicy::LegacySingleWinnerFee if count == 1 => amount / 10,
                    _ => 0,
                };
                let distributable = amount - fee;
                let mut payouts = [0; SEATS];
                for (priority, winner) in winners.into_iter().enumerate() {
                    payouts[winner] = distributable / count
                        + u32::from((priority as u32) < distributable % count);
                }
                PotAward {
                    amount,
                    fee,
                    eligible,
                    payouts,
                }
            })
            .collect::<Vec<_>>();
        for seat in 0..SEATS {
            self.stacks[seat] +=
                uncalled[seat] + pots.iter().map(|pot| pot.payouts[seat]).sum::<u32>();
        }
        self.actor = None;
        self.bets = [0; SEATS];
        self.contributions = [0; SEATS];
        self.outcome = Some(Outcome {
            pots,
            uncalled,
            stacks: self.stacks,
            delta: std::array::from_fn(|seat| {
                i64::from(self.stacks[seat]) - i64::from(self.starting[seat])
            }),
            showdown,
            revealed: std::array::from_fn(|seat| {
                ranks[seat].map(|_| self.hole[seat].map(|card| card.to_string()))
            }),
        });
    }

    fn pot_layers(&self) -> (Vec<(u32, [bool; SEATS])>, [u32; SEATS]) {
        let mut thresholds = self.contributions;
        thresholds.sort_unstable();
        let mut layers: Vec<(u32, [bool; SEATS])> = Vec::new();
        let mut uncalled = [0; SEATS];
        let mut previous = 0;
        for threshold in thresholds.into_iter().filter(|&threshold| threshold != 0) {
            if threshold == previous {
                continue;
            }
            let width = threshold - previous;
            previous = threshold;
            let paid = self.contributions.map(|amount| amount >= threshold);
            let count = paid.iter().filter(|&&paid| paid).count() as u32;
            if count == 1 {
                uncalled[paid.iter().position(|&paid| paid).unwrap()] = width;
                break;
            }
            let amount = width * count;
            let eligible = std::array::from_fn(|seat| paid[seat] && !self.folded[seat]);
            // Folded stacks add money but cannot create a new pot when they
            // leave the eligible set unchanged. Split odd chips only once.
            if let Some((total, _)) = layers.last_mut().filter(|(_, seats)| *seats == eligible) {
                *total += amount;
            } else {
                layers.push((amount, eligible));
            }
        }
        (layers, uncalled)
    }
}

fn clockwise(after: usize, players: usize) -> impl Iterator<Item = usize> {
    (1..=players).map(move |offset| (after + offset) % players)
}

#[cfg(test)]
mod tests;
