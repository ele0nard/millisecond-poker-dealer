use super::*;
use rand::{Rng, SeedableRng, rngs::StdRng};
use std::collections::HashSet;

fn act(hand: &mut Hand, seat: usize, action: Move) {
    assert_eq!(hand.actor(), Some(seat));
    assert!(hand.play(seat, action), "{action:?} was rejected");
}

#[test]
fn validates_setup_and_runs_each_street_in_position_order() {
    assert!(matches!(
        Hand::new([0; 32], 6, [1000; 6]),
        Err(SetupError::InvalidButton)
    ));
    assert!(matches!(
        Hand::new([0; 32], 0, [0, 1, 1, 1, 1, 1]),
        Err(SetupError::EmptyStack)
    ));
    assert!(matches!(
        Hand::new([0; 32], 0, [u32::MAX, 1, 1, 1, 1, 1]),
        Err(SetupError::ChipTotalOverflow)
    ));
    let mut hand = Hand::new([1; 32], 4, [1000; 6]).unwrap();
    assert_eq!(hand.view().bets, [10, 0, 0, 0, 0, 5]);
    assert_eq!(hand.view().pot, 15);
    let before = hand.observation();
    assert!(!hand.play(6, Move::Call));
    assert!(!hand.play(0, Move::Check));
    assert!(!hand.play(1, Move::Raise { to: 19 }));
    assert_eq!(hand.observation(), before);
    assert_eq!(hand.timeout(), Some(Move::Fold));
    for seat in [1, 2, 3, 4, 5] {
        act(&mut hand, seat, Move::Call);
    }
    assert_eq!(hand.legal().unwrap().raise, Some([20, 1000]));
    assert_eq!(hand.timeout(), Some(Move::Check));
    act(&mut hand, 0, Move::Check);

    for (street, count) in [(Street::Flop, 3), (Street::Turn, 4), (Street::River, 5)] {
        assert_eq!(hand.view().street, street);
        assert_eq!(hand.view().board.len(), count);
        assert_eq!(hand.view().pot, 60);
        assert_eq!(hand.view().bets, [0; 6]);
        for seat in [5, 0, 1, 2, 3, 4] {
            act(&mut hand, seat, Move::Check);
        }
    }
    let outcome = hand.outcome().unwrap();
    assert!(outcome.showdown);
    assert!(outcome.revealed.iter().all(Option::is_some));
    assert_eq!(hand.view().pot, 0);
    assert!(hand.observation().is_none());
    assert!(hand.timeout().is_none());
    assert!(!hand.play(0, Move::Check));
}

#[test]
fn short_big_blind_preserves_the_bring_in_and_skips_the_all_in_seat() {
    let mut hand = Hand::new([2; 32], 0, [100, 100, 3, 100, 100, 100]).unwrap();
    assert_eq!(hand.view().bets, [0, 5, 3, 0, 0, 0]);
    assert_eq!(hand.legal().unwrap().call, Some(10));
    assert_eq!(hand.legal().unwrap().raise, Some([20, 100]));
    for seat in [3, 4, 5, 0] {
        act(&mut hand, seat, Move::Fold);
    }
    // The small blind already covered the actual big blind. There is no
    // action left against the all-in seat, and the excess 2 is returned.
    let outcome = hand.outcome().unwrap();
    assert!(outcome.showdown);
    assert_eq!(outcome.uncalled, [0, 2, 0, 0, 0, 0]);
    assert_eq!(outcome.pots[0].amount, 6);
    assert_eq!(hand.view().board.len(), 5);
}

#[test]
fn cumulative_short_raises_reopen_per_seat_at_the_full_increment() {
    for (short_total, expected_raise) in [(39, None), (40, Some([55, 200]))] {
        let mut hand = Hand::new([3; 32], 0, [short_total, 200, 200, 200, 30, 200]).unwrap();
        act(&mut hand, 3, Move::Raise { to: 25 }); // full raise of 15
        assert_eq!(hand.legal().unwrap().raise, Some([30, 30]));
        assert!(!hand.play(4, Move::Raise { to: 29 }));
        act(&mut hand, 4, Move::Raise { to: 30 }); // short by 10
        act(&mut hand, 5, Move::Call); // seat 5 last faced 30
        act(&mut hand, 0, Move::Raise { to: short_total });
        act(&mut hand, 1, Move::Call);
        act(&mut hand, 2, Move::Call);
        assert_eq!(hand.actor(), Some(3));
        assert_eq!(hand.legal().unwrap().raise, expected_raise);
        act(&mut hand, 3, Move::Call);
        assert_eq!(hand.actor(), Some(5));
        assert_eq!(hand.legal().unwrap().raise, None);
        act(&mut hand, 5, Move::Call);
        assert_eq!(hand.view().street, Street::Flop);
    }
}

#[test]
fn full_reraise_sets_the_increment_for_previous_callers() {
    let mut hand = Hand::new([7; 32], 0, [1000; 6]).unwrap();
    act(&mut hand, 3, Move::Raise { to: 30 });
    act(&mut hand, 4, Move::Call);
    act(&mut hand, 5, Move::Raise { to: 75 });
    for seat in [0, 1, 2] {
        act(&mut hand, seat, Move::Fold);
    }
    assert_eq!(hand.legal().unwrap().raise, Some([120, 1000]));
    act(&mut hand, 3, Move::Call);
    assert_eq!(hand.legal().unwrap().raise, Some([120, 1000]));
}

#[test]
fn short_postflop_open_does_not_reopen_a_check() {
    let mut hand = Hand::new([4; 32], 0, [100, 100, 15, 100, 100, 100]).unwrap();
    for seat in [3, 4, 5, 0, 1] {
        act(&mut hand, seat, Move::Call);
    }
    act(&mut hand, 2, Move::Check);
    act(&mut hand, 1, Move::Check);
    assert_eq!(hand.legal().unwrap().raise, Some([5, 5]));
    act(&mut hand, 2, Move::Raise { to: 5 });
    assert_eq!(hand.legal().unwrap().raise, Some([15, 90]));
    for seat in [3, 4, 5, 0] {
        act(&mut hand, seat, Move::Call);
    }
    assert_eq!(hand.legal().unwrap().raise, None);
    act(&mut hand, 1, Move::Call);
    assert_eq!(hand.view().street, Street::Turn);
}

fn side_pot_hand(board: [u8; 5], holes: [[u8; 2]; 6]) -> Hand {
    let mut hand = Hand::new([8; 32], 0, [10, 21, 21, 21, 21, 21]).unwrap();
    hand.hole = holes.map(|cards| cards.map(Card));
    let [a, b, c, d, e] = board;
    hand.deck = [1, a, b, c, 2, d, 3, e]
        .into_iter()
        .rev()
        .map(Card)
        .collect();
    act(&mut hand, 3, Move::Raise { to: 21 });
    act(&mut hand, 4, Move::Call);
    act(&mut hand, 5, Move::Fold);
    assert_eq!(hand.legal().unwrap().call, Some(10));
    act(&mut hand, 0, Move::Call);
    act(&mut hand, 1, Move::Fold); // contributes a dead small blind
    assert_eq!(hand.legal().unwrap().raise, None); // no opponent can respond
    act(&mut hand, 2, Move::Call);
    hand
}

#[test]
fn side_pots_pay_only_eligible_players_and_exclude_folded_cards() {
    let hand = side_pot_hand(
        [0, 16, 32, 48, 9],
        [[22, 35], [10, 23], [11, 24], [12, 25], [8, 21], [7, 20]],
    );
    let outcome = hand.outcome().unwrap();
    assert_eq!(outcome.uncalled, [0; 6]);
    assert_eq!(outcome.pots.len(), 2);
    assert_eq!(outcome.pots[0].amount, 45);
    assert_eq!(outcome.pots[0].payouts, [45, 0, 0, 0, 0, 0]);
    assert_eq!(outcome.pots[1].amount, 33);
    assert_eq!(
        outcome.pots[1].eligible,
        [false, false, true, true, true, false]
    );
    assert_eq!(outcome.pots[1].payouts, [0, 0, 0, 33, 0, 0]);
    assert_eq!(outcome.stacks, [45, 16, 0, 33, 0, 21]);
    assert_eq!(outcome.revealed[1], None);
    assert_eq!(outcome.revealed[5], None);
}

#[test]
fn sole_main_pot_winner_and_tied_side_pot_settle_independently() {
    // Four kings on the board. Seat zero has an ace, seats two and three have queens.
    let hand = side_pot_hand(
        [11, 24, 37, 50, 0],
        [[12, 1], [5, 6], [10, 2], [23, 3], [9, 4], [7, 8]],
    );
    let pots = &hand.outcome().unwrap().pots;
    assert_eq!((pots[0].fee, pots[0].payouts), (0, [45, 0, 0, 0, 0, 0]));
    assert_eq!((pots[1].fee, pots[1].payouts), (0, [0, 0, 17, 16, 0, 0]));
}

#[test]
fn six_all_ins_can_create_a_main_pot_and_four_side_pots() {
    let mut hand = Hand::new([11; 32], 0, [20, 40, 60, 120, 120, 10]).unwrap();
    act(&mut hand, 3, Move::Raise { to: 120 });
    for seat in [4, 5, 0, 1, 2] {
        act(&mut hand, seat, Move::Call);
    }
    let outcome = hand.outcome().unwrap();
    assert_eq!(
        outcome
            .pots
            .iter()
            .map(|pot| pot.amount)
            .collect::<Vec<_>>(),
        [60, 50, 80, 60, 120]
    );
    assert_eq!(
        outcome.pots[4].eligible,
        [false, false, false, true, true, false]
    );
    assert_eq!(outcome.uncalled, [0; 6]);
    assert_eq!(outcome.stacks.iter().sum::<u32>(), 370);
}

#[test]
fn split_pots_award_odd_chips_once_per_pot_in_clockwise_order() {
    let hand = side_pot_hand(
        [47, 48, 49, 50, 51], // board is a royal flush
        [[4, 5], [6, 7], [8, 9], [10, 11], [12, 13], [14, 15]],
    );
    let outcome = hand.outcome().unwrap();
    assert_eq!(outcome.pots[0].payouts, [11, 0, 12, 11, 11, 0]);
    assert_eq!(outcome.pots[1].payouts, [0, 0, 11, 11, 11, 0]);
    assert_eq!(outcome.delta.iter().sum::<i64>(), 0);
}

#[test]
fn folding_to_a_single_winner_returns_uncalled_chips_without_showdown() {
    let mut hand = Hand::new([1; 32], 0, [100; 6]).unwrap();
    act(&mut hand, 3, Move::Raise { to: 100 });
    for seat in [4, 5, 0, 1, 2] {
        act(&mut hand, seat, Move::Fold);
    }
    let outcome = hand.outcome().unwrap();
    assert!(!outcome.showdown);
    assert!(outcome.revealed.iter().all(Option::is_none));
    assert!(hand.view().board.is_empty());
    assert_eq!(outcome.uncalled, [0, 0, 0, 90, 0, 0]);
    assert_eq!(outcome.pots[0].payouts, [0, 0, 0, 25, 0, 0]);
    assert_eq!(outcome.delta, [0, -5, -10, 15, 0, 0]);
}

#[test]
fn actor_projection_ignores_opponent_cards_and_future_board() {
    let mut hand = Hand::new([8; 32], 0, [100; 6]).unwrap();
    let before = hand.observation().unwrap();
    assert_eq!(before.seat, 3);
    assert!(before.table.board.is_empty());
    let next_card = hand.deck.last_mut().unwrap();
    std::mem::swap(next_card, &mut hand.hole[4][0]);
    assert_eq!(hand.observation(), Some(before));
}

#[test]
fn recorded_rules_replay_each_fee_era() {
    for (rules, has_fee, expected_fee) in [(1, false, 0), (1, true, 1), (2, true, 0)] {
        let policy = PotPolicy::recorded(rules, has_fee).unwrap();
        let mut hand = Hand::with_pot_policy([42; 32], 0, &[1000, 1000], policy).unwrap();
        act(&mut hand, 0, Move::Fold);
        let outcome = hand.outcome().unwrap();
        assert_eq!(outcome.total_fee(), expected_fee);
        assert_eq!(outcome.stacks[1], 1005 - expected_fee);
    }
    assert!(PotPolicy::recorded(3, true).is_none());
}

#[test]
fn seeded_action_sequences_replay_and_conserve_every_chip() {
    for sample in 0..512u64 {
        let mut policy = StdRng::seed_from_u64(sample);
        let stacks = std::array::from_fn(|_| policy.random_range(1..=120));
        let seed: [u8; 32] = policy.random();
        let button = policy.random_range(0..SEATS);
        let mut hand = Hand::new(seed, button, stacks).unwrap();
        let mut replay = Hand::new(seed, button, stacks).unwrap();
        let total = stacks.iter().sum::<u32>();
        let mut steps = 0;
        let mut final_contributions = hand.view().contributions.iter().sum::<u32>();
        while let Some(observation) = hand.observation() {
            assert_eq!(replay.observation().as_ref(), Some(&observation));
            assert_eq!(
                observation.table.stacks.iter().sum::<u32>() + observation.table.pot,
                total
            );
            let legal = observation.legal;
            let mut actions = Vec::new();
            if legal.check {
                actions.push(Move::Check);
            } else {
                actions.extend([Move::Fold, Move::Call]);
            }
            if let Some([min, max]) = legal.raise {
                actions.extend([Move::Raise { to: min }, Move::Raise { to: max }]);
            }
            let action = actions[policy.random_range(0..actions.len())];
            final_contributions = observation.table.pot
                + match action {
                    Move::Call => legal.call.unwrap(),
                    Move::Raise { to } => to - observation.table.bets[observation.seat],
                    _ => 0,
                };
            act(&mut hand, observation.seat, action);
            act(&mut replay, observation.seat, action);
            steps += 1;
            assert!(steps < 256, "hand failed to terminate for sample {sample}");
        }
        let outcome = hand.outcome().unwrap();
        assert_eq!(Some(outcome), replay.outcome());
        assert_eq!(hand.view(), replay.view());
        assert_eq!(outcome.stacks.iter().sum::<u32>(), total);
        assert_eq!(outcome.delta.iter().sum::<i64>(), 0);
        assert_eq!(
            outcome.pots.iter().map(|pot| pot.amount).sum::<u32>()
                + outcome.uncalled.iter().sum::<u32>(),
            final_contributions,
        );
        for pot in &outcome.pots {
            assert_eq!(pot.fee, 0);
            assert_eq!(pot.payouts.iter().sum::<u32>(), pot.amount);
            for (seat, &payout) in pot.payouts.iter().enumerate() {
                assert!(payout == 0 || (pot.eligible[seat] && !hand.folded[seat]));
            }
        }
        let unique_cards: HashSet<_> = hand
            .hole
            .iter()
            .flatten()
            .chain(hand.board.iter())
            .map(|card| card.0)
            .collect();
        assert_eq!(unique_cards.len(), 12 + hand.board.len());
    }
}

#[test]
fn heads_up_button_posts_small_and_acts_first_only_preflop() {
    let mut hand = Hand::with_stacks([8; 32], 1, &[1000, 1000]).unwrap();
    assert_eq!(hand.view().bets, [10, 5, 0, 0, 0, 0]);
    assert_eq!(hand.view().folded, [false, false, true, true, true, true]);
    assert_eq!(hand.actor(), Some(1));
    assert!(hand.play(1, Move::Call));
    assert!(hand.play(0, Move::Check));
    assert_eq!(hand.view().street, Street::Flop);
    assert_eq!(hand.actor(), Some(0));
}

#[test]
fn every_supported_size_conserves_chips_without_dealing_to_empty_seats() {
    for count in MIN_SEATS..=SEATS {
        let mut hand = Hand::with_stacks([5; 32], count - 1, &[1000; SEATS][..count]).unwrap();
        while let Some(actor) = hand.actor() {
            assert!(actor < count);
            let action = if hand.legal().unwrap().check {
                Move::Check
            } else {
                Move::Call
            };
            assert!(hand.play(actor, action));
        }
        let outcome = hand.outcome().unwrap();
        assert!(outcome.showdown);
        assert_eq!(outcome.stacks.iter().sum::<u32>(), (count as u32) * 1000);
        assert!(outcome.stacks[count..].iter().all(|&stack| stack == 0));
        assert!(outcome.revealed[count..].iter().all(Option::is_none));
    }
}
