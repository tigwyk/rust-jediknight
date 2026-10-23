//! The force-power economy: the bins the engine spends, and the ladder it walks.
//!
//! Recovered from `JK.EXE`: `jkPlayer_GetJediRank` `0x4074d0`, `jkPlayer_CalcAlignment`
//! `0x405cf0`, `jkPlayer_SetAccessiblePowers` `0x406860`, `jkPlayer_SetProtectionDeadlysight`
//! `0x407040`, `jkPlayer_DisallowOtherSide` `0x407210`, and `jkGuiForce_Show` `0x416480`, which
//! is the screen that spends the points. The five floats the arithmetic uses sit at `0x522054` and
//! were read out of the executable: `0.0`, `100.0`, `-100.0`, `6.25`, `-6.25`, `-20.0`, `4.0`.
//!
//! A Jedi's powers are ordinary inventory bins — the same `items.dat` rows `hud::items` parses —
//! so everything here works on the player's bin map and nothing needs a private model:
//!
//! ```text
//! 0x10 / 0x11  force points: newly awarded / unspent pool (the screen merges 0x10 into 0x11)
//! 0x12         the side the player declared: 1 = light, 2 = dark
//! 0x14         jedi rank
//! 0x15..0x18   the four basic powers
//! 0x19..0x1c   the four light-side powers
//! 0x1d         protection (the light mastery)
//! 0x1e..0x21   the four dark-side powers
//! 0x22         deadly sight (the dark mastery)
//! 0x48 / 0x49  light / dark side points, summed at the tally screen
//! 0x4b         the alignment the game stores: -100..100
//! ```

use std::collections::BTreeMap;

/// Force points a level has just awarded, not yet folded into the pool.
pub const POINTS_NEW: i32 = 0x10;
/// Force points the player has to spend.
pub const POINTS: i32 = 0x11;
/// The side the player declared at character creation: 1 light, 2 dark, 0 undecided.
pub const SIDE: i32 = 0x12;
/// `jkPlayer_GetJediRank` is exactly this bin.
pub const RANK: i32 = 0x14;
/// The four powers every Jedi gets.
pub const BASICS: [i32; 4] = [0x15, 0x16, 0x17, 0x18];
/// The four light-side powers, in the order `SetAccessiblePowers` unlocks them.
pub const LIGHT: [i32; 4] = [0x19, 0x1a, 0x1b, 0x1c];
/// The four dark-side powers, unlocked in step with [`LIGHT`].
pub const DARK: [i32; 4] = [0x1e, 0x1f, 0x20, 0x21];
/// The light mastery, granted at rank 8 to a Jedi who put nothing into the basics.
pub const PROTECTION: i32 = 0x1d;
/// The dark mastery, the mirror of [`PROTECTION`].
pub const DEADLYSIGHT: i32 = 0x22;
/// Light side points, accumulated by the tally screen.
pub const SIDE_LIGHT: i32 = 0x48;
/// Dark side points, accumulated by the tally screen.
pub const SIDE_DARK: i32 = 0x49;
/// Where `CalcAlignment` leaves its answer.
pub const ALIGNMENT: i32 = 0x4b;

/// `_DAT_0052206c`: what one dark level costs on the alignment scale.
const WEIGHT_DARK: f32 = 6.25;
/// `_DAT_00522070`: what one light level is worth. The engine subtracts both, so this one is
/// negative and a light level moves the sum up.
const WEIGHT_LIGHT: f32 = -6.25;
/// `_DAT_00522074`: the tally screen's head start toward the light.
const TALLY_BIAS: f32 = -20.0;
/// `_DAT_00522078`: every power on a side must be at least this to earn that side's mastery.
const MASTERY_LEVEL: f32 = 4.0;
/// `_DAT_00522064` / `_DAT_00522068`: the ends of the alignment scale.
const ALIGNMENT_MAX: f32 = 100.0;
const ALIGNMENT_MIN: f32 = -100.0;
/// `_DAT_00522054`: the "above zero" test every loop in those functions uses.
const NONE: f32 = 0.0;

fn amount(inv: &BTreeMap<i32, f32>, bin: i32) -> f32 {
    inv.get(&bin).copied().unwrap_or(0.0)
}

fn level(inv: &BTreeMap<i32, f32>, bin: i32) -> i32 {
    amount(inv, bin) as i32
}

/// Which force bins the screen lets the player put points into, for a given rank.
///
/// `jkPlayer_SetAccessiblePowers`: clear the available flag on every force bin, then set it on the
/// four basics for any rank above zero, and on one light power and its dark twin per rank above
/// three. The protection/deadly-sight mastery is *not* here — it has its own function.
pub fn accessible_powers(rank: i32) -> Vec<i32> {
    let mut bins = Vec::new();
    if rank > 0 {
        bins.extend(BASICS);
    }
    for (tier, (light, dark)) in LIGHT.iter().zip(DARK.iter()).enumerate() {
        if rank > 3 + tier as i32 {
            bins.push(*light);
            bins.push(*dark);
        }
    }
    bins
}

/// The player's alignment: positive is the light, negative the dark, clamped to +/-100.
///
/// Every dark level pulls the sum down and every light level pushes it up, at `6.25` a level. A
/// declared side short-circuits the sum entirely — `jkPlayer_CalcAlignment` returns the end of the
/// scale for bins `0x12 == 1/2` before it adds anything. `tally` is the engine's `param_1 == 0`
/// case: the post-level tally screen passes zero and gets the two side-point totals folded in, as
/// a ratio of the light total to the dark one plus the `-20` that every new game starts with.
pub fn alignment(inv: &BTreeMap<i32, f32>, tally: bool) -> f32 {
    match level(inv, SIDE) {
        1 => return ALIGNMENT_MAX,
        2 => return ALIGNMENT_MIN,
        _ => {}
    }
    let mut sum = 0.0;
    for bin in DARK {
        sum -= amount(inv, bin) * WEIGHT_DARK;
    }
    for bin in LIGHT {
        sum -= amount(inv, bin) * WEIGHT_LIGHT;
    }
    if tally {
        let lit = amount(inv, SIDE_LIGHT);
        let dark = amount(inv, SIDE_DARK);
        sum = if dark <= NONE {
            sum - TALLY_BIAS
        } else {
            (sum - (lit / dark) * ALIGNMENT_MAX) - TALLY_BIAS
        };
    }
    sum.clamp(ALIGNMENT_MIN, ALIGNMENT_MAX)
}

/// `jkPlayer_SetProtectionDeadlysight`: the rank-8 mastery. Returns 1 for protection, 2 for deadly
/// sight, 0 for neither.
///
/// The test is deliberately odd and the port keeps it: the mastery needs *all four* powers of one
/// side at [`MASTERY_LEVEL`], *none* of the other side, and nothing at all in the four basics.
pub fn protection_deadlysight(inv: &mut BTreeMap<i32, f32>) -> i32 {
    const RANK_FOR_MASTERY: i32 = 8;
    let side_clear = |inv: &BTreeMap<i32, f32>, bins: [i32; 4]| {
        bins.iter().all(|bin| amount(inv, *bin) <= NONE)
    };
    let side_maxed = |inv: &BTreeMap<i32, f32>, bins: [i32; 4]| {
        bins.iter().all(|bin| amount(inv, *bin) >= MASTERY_LEVEL)
    };
    let basics_clear = side_clear(inv, BASICS);
    let mut granted = 0;
    if level(inv, RANK) == RANK_FOR_MASTERY {
        if side_maxed(inv, LIGHT) && side_clear(inv, DARK) && basics_clear {
            inv.insert(PROTECTION, MASTERY_LEVEL);
            inv.insert(DEADLYSIGHT, 0.0);
            granted = 1;
        } else if side_maxed(inv, DARK) && side_clear(inv, LIGHT) && basics_clear {
            inv.insert(DEADLYSIGHT, MASTERY_LEVEL);
            inv.insert(PROTECTION, 0.0);
            granted = 2;
        } else {
            inv.insert(PROTECTION, 0.0);
            inv.insert(DEADLYSIGHT, 0.0);
        }
    }
    granted
}

/// `jkPlayer_DisallowOtherSide`: once the player is past rank 6 the side they are *not* on is
/// hidden, unless the alignment sits exactly on the fence and then both stay open.
///
/// Returns `(light, dark)`.
pub fn allowed_sides(inv: &BTreeMap<i32, f32>, rank: i32) -> (bool, bool) {
    const RANK_BOTH_SIDES: i32 = 6;
    if rank <= RANK_BOTH_SIDES {
        return (true, true);
    }
    let mut sum = 0.0;
    for bin in DARK {
        sum -= amount(inv, bin) * WEIGHT_DARK;
    }
    for bin in LIGHT {
        sum -= amount(inv, bin) * WEIGHT_LIGHT;
    }
    if sum > NONE {
        (true, false)
    } else if sum < NONE {
        (false, true)
    } else {
        (true, true)
    }
}

/// `jkPlayer_FixStars`: reconcile the unspent pool against the rank's own budget, and return the
/// pool it left behind.
///
/// A Jedi's total is **three points per rank** (`rank * 3`). What counts as spent is the unspent
/// pool plus every power level from `0x15` up to `0x22` — the loop steps `0x14..0x23` and skips the
/// rank itself, protection and deadly sight, which are not bought. A Jedi under budget is topped up
/// to it; one over budget is refunded a point at a time in the engine's own order: the pool first,
/// then the highest tier of each side down to the basics, dark before light inside every pair.
///
/// `FixStars` spins forever if it is over budget with every bin already empty; that cannot happen
/// (the spend it counted is the same set it drains), but this stops rather than hangs if it ever does.
pub fn fix_stars(inv: &mut BTreeMap<i32, f32>) -> i32 {
    /// The order `FixStars` drains in when a Jedi is over budget.
    const REFUND: [i32; 13] = [
        POINTS,
        DARK[3],
        LIGHT[3],
        DARK[2],
        LIGHT[2],
        DARK[1],
        LIGHT[1],
        DARK[0],
        LIGHT[0],
        BASICS[3],
        BASICS[2],
        BASICS[1],
        BASICS[0],
    ];
    let rank = level(inv, RANK);
    let budget = rank * 3;
    let spent: i32 = level(inv, POINTS)
        + BASICS
            .iter()
            .chain(LIGHT.iter())
            .chain(DARK.iter())
            .map(|bin| level(inv, *bin))
            .sum::<i32>();
    if spent < budget {
        let pool = (budget - spent) as f32;
        inv.insert(POINTS, pool);
        return pool as i32;
    }
    let mut over = spent - budget;
    while over > 0 {
        // `FixStars` walks the same list for every point, so the pool drains before any power.
        let Some(bin) = REFUND.iter().find(|bin| level(inv, **bin) > 0) else {
            break;
        };
        inv.insert(*bin, (level(inv, *bin) - 1) as f32);
        over -= 1;
    }
    level(inv, POINTS)
}

/// `jkMain_EndLevel`: the powers a finished level grants, from the episode row that names them.
///
/// An episode row carries a light bin and a dark bin (`.epi` columns 5 and 6, `lightpow`/`darkpow`
/// in `src/episode.rs`). `jkMain_EndLevel` grants each one whose bin sits strictly between `0x13` and
/// `0x23`, through `sithInventory_SetCarries` — which sets the *same* available bit
/// `SetAccessiblePowers` sets — and skips a light grant for a player who declared the dark side and
/// the dark grant for one who declared the light. `choice` is `jkPlayer_GetChoice()`: 1 light, 2 dark.
///
/// This is how single-player Jedi actually receive powers: level 3's row names bin `22` (`f_speed`)
/// and the row's own comment says `[Speed]`, so a light-side player has force speed from level 3 on.
pub fn episode_grants(lightpow: i32, darkpow: i32, choice: i32) -> Vec<i32> {
    let in_range = |bin: i32| bin > 0x13 && bin < 0x23;
    let mut bins = Vec::new();
    if in_range(lightpow) && choice != 2 {
        bins.push(lightpow);
    }
    if in_range(darkpow) && choice != 1 {
        bins.push(darkpow);
    }
    bins
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inv(pairs: &[(i32, f32)]) -> BTreeMap<i32, f32> {
        pairs.iter().copied().collect()
    }

    /// `SetAccessiblePowers` walks the ranks: nothing at rank 0, the four basics at rank 1, and one
    /// light power plus its dark twin for each rank above three.
    #[test]
    fn the_rank_ladder_unlocks_powers_in_pairs() {
        assert!(accessible_powers(0).is_empty());
        assert_eq!(accessible_powers(1), BASICS.to_vec());
        assert_eq!(accessible_powers(3), BASICS.to_vec(), "rank 3 is still only the basics");
        assert_eq!(
            accessible_powers(4),
            [BASICS.to_vec(), vec![LIGHT[0], DARK[0]]].concat()
        );
        let rank_7 = accessible_powers(7);
        assert_eq!(rank_7.len(), 12);
        for bin in BASICS.iter().chain(LIGHT.iter()).chain(DARK.iter()) {
            assert!(rank_7.contains(bin), "rank 7 is missing bin {bin:#x}");
        }
        assert!(accessible_powers(8).len() == 12, "the ladder stops at rank 7");
    }

    /// A level of a dark power is `-6.25` and a light one `+6.25`, and the sum is clamped.
    #[test]
    fn the_alignment_weighs_both_sides_and_clamps() {
        assert_eq!(alignment(&BTreeMap::new(), false), 0.0);
        let dark = inv(&[(DARK[0], 4.0)]);
        assert_eq!(alignment(&dark, false), -25.0);
        let light = inv(&[(LIGHT[0], 4.0)]);
        assert_eq!(alignment(&light, false), 25.0);
        let both = inv(&[(LIGHT[0], 4.0), (DARK[0], 4.0)]);
        assert_eq!(alignment(&both, false), 0.0, "a level each way cancels");
        let wall = inv(&[(DARK[0], 4.0), (DARK[1], 4.0), (DARK[2], 4.0), (DARK[3], 4.0)]);
        assert_eq!(alignment(&wall, false), -100.0, "16 dark levels sit exactly on the floor");
    }

    /// A declared side short-circuits the sum, and the tally screen folds in its side points.
    #[test]
    fn a_declared_side_wins_and_the_tally_adds_its_bias() {
        let declared = inv(&[(SIDE, 1.0), (DARK[0], 4.0)]);
        assert_eq!(alignment(&declared, false), 100.0, "the declaration beats the powers");
        let dark_declared = inv(&[(SIDE, 2.0), (LIGHT[0], 4.0)]);
        assert_eq!(alignment(&dark_declared, false), -100.0);
        assert_eq!(
            alignment(&BTreeMap::new(), true),
            20.0,
            "a tally with no side points is the -20 bias, not zero"
        );
        let split = inv(&[(SIDE_LIGHT, 4.0), (SIDE_DARK, 4.0)]);
        assert_eq!(alignment(&split, true), -80.0);
    }

    /// The mastery needs rank 8, one side maxed, the other empty and the basics untouched.
    #[test]
    fn the_mastery_power_needs_a_pure_rank_eight_jedi() {
        let mut jedi = inv(&[
            (RANK, 8.0),
            (LIGHT[0], 4.0),
            (LIGHT[1], 4.0),
            (LIGHT[2], 4.0),
            (LIGHT[3], 4.0),
        ]);
        assert_eq!(protection_deadlysight(&mut jedi), 1);
        assert_eq!(jedi.get(&PROTECTION).copied(), Some(4.0));
        assert_eq!(jedi.get(&DEADLYSIGHT).copied(), Some(0.0));

        let mut sith = inv(&[
            (RANK, 8.0),
            (DARK[0], 4.0),
            (DARK[1], 4.0),
            (DARK[2], 4.0),
            (DARK[3], 4.0),
        ]);
        assert_eq!(protection_deadlysight(&mut sith), 2);
        assert_eq!(sith.get(&DEADLYSIGHT).copied(), Some(4.0));

        // A basic power spent on the way is enough to lose it.
        let mut spent = inv(&[
            (RANK, 8.0),
            (LIGHT[0], 4.0),
            (LIGHT[1], 4.0),
            (LIGHT[2], 4.0),
            (LIGHT[3], 4.0),
            (BASICS[0], 1.0),
        ]);
        assert_eq!(protection_deadlysight(&mut spent), 0);
        assert_eq!(spent.get(&PROTECTION).copied(), Some(0.0));

        let mut mixed = inv(&[(RANK, 8.0), (LIGHT[0], 4.0), (DARK[0], 4.0)]);
        assert_eq!(protection_deadlysight(&mut mixed), 0, "a foot in both camps earns neither");
    }

    /// Past rank 6 the opposite side is hidden; on the fence both stay open.
    #[test]
    fn a_high_rank_jedi_loses_the_other_side() {
        let light = inv(&[(LIGHT[0], 4.0)]);
        assert_eq!(allowed_sides(&light, 6), (true, true), "rank 6 still sees both");
        assert_eq!(allowed_sides(&light, 7), (true, false));
        let dark = inv(&[(DARK[0], 4.0)]);
        assert_eq!(allowed_sides(&dark, 7), (false, true));
        assert_eq!(allowed_sides(&BTreeMap::new(), 7), (true, true), "neutral keeps both");
    }

    /// `FixStars`'s budget is three points a rank, and a Jedi under it is topped up to it.
    #[test]
    fn the_point_pool_is_three_per_rank() {
        let mut fresh = inv(&[(RANK, 3.0)]);
        assert_eq!(fix_stars(&mut fresh), 9);
        assert_eq!(fresh.get(&POINTS).copied(), Some(9.0));

        let mut spent = inv(&[(RANK, 3.0), (BASICS[0], 2.0), (LIGHT[1], 1.0)]);
        assert_eq!(fix_stars(&mut spent), 6, "two in jump and one in a power leaves six");
        assert_eq!(spent.get(&POINTS).copied(), Some(6.0));

        let mut none = inv(&[(RANK, 0.0)]);
        assert_eq!(fix_stars(&mut none), 0, "a rank-0 Jedi has nothing to spend");
    }

    /// Over budget the pool drains first, then the powers in the engine's own order.
    #[test]
    fn an_over_budget_jedi_is_refunded_pool_first() {
        let mut over = inv(&[(RANK, 1.0), (POINTS, 1.0), (DARK[3], 3.0)]);
        assert_eq!(fix_stars(&mut over), 0, "one point over budget comes out of the pool");
        assert_eq!(over.get(&DARK[3]).copied(), Some(3.0), "the power is untouched");

        let mut powers = inv(&[(RANK, 1.0), (DARK[3], 2.0), (LIGHT[0], 2.0)]);
        assert_eq!(fix_stars(&mut powers), 0);
        assert_eq!(
            powers.get(&DARK[3]).copied(),
            Some(1.0),
            "the pool is empty, so the top tier drains first, dark before light"
        );
        assert_eq!(powers.get(&LIGHT[0]).copied(), Some(2.0));
    }

    /// The mastery bins are granted, not bought, so they never count against the budget.
    #[test]
    fn the_mastery_bins_are_outside_the_budget() {
        let mut master = inv(&[(RANK, 1.0), (PROTECTION, 4.0), (DEADLYSIGHT, 4.0)]);
        assert_eq!(fix_stars(&mut master), 3);
        assert_eq!(master.get(&PROTECTION).copied(), Some(4.0), "and they are left alone");
    }

    /// A level grants the power its episode row names, and only to the side that may use it.
    #[test]
    fn an_episode_row_grants_the_power_it_names() {
        // Level 3's row: `22 0`, and its comment says `[Speed]`.
        assert_eq!(episode_grants(22, 0, 0), vec![22], "an undecided player gets the light power");
        assert_eq!(episode_grants(22, 0, 1), vec![22]);
        assert_eq!(episode_grants(22, 0, 2), Vec::<i32>::new(), "a dark Jedi is not given it");
        assert_eq!(episode_grants(0, 30, 1), Vec::<i32>::new());
        assert_eq!(episode_grants(0, 30, 2), vec![30]);
        assert_eq!(
            episode_grants(19, 34, 0),
            vec![34],
            "the range excludes 0x13 itself but still includes deadly sight"
        );
        assert_eq!(episode_grants(20, 0, 0), vec![20], "the rank bin itself is in range");
        assert_eq!(episode_grants(0, 0, 0), Vec::<i32>::new(), "a row with no powers grants none");
    }
}
