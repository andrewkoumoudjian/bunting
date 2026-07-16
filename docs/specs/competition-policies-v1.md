# Bunting competition MVC policies v1

Status: implemented Bunting-native policies; no RIT equivalence is claimed.

All money is signed `i128` minor currency units, prices are signed `i64` ticks,
quantities are signed `i64` lots, and arithmetic is checked. A command that
overflows rejects before origin commit. The run snapshot pins the simulation
policy version, while every participant projection carries these stable policy
identities.

## `bunting.pnl.v1`

For each instrument, unrealized P&L is `mark_value - cost_basis`, where
`mark_value = price_ticks * settled_quantity_lots` in exact integer units.
Realized P&L is the committed portfolio-ledger value. Net liquidation value in
one currency is settled cash plus accrued cash plus scheduled cash plus fees,
minus margin, plus realized and unrealized P&L over the participant's positions.
No cross-currency conversion is inferred.

## `bunting.commission.zero.v1`

The competition MVC charges zero commission and zero rebate. The fee balance
therefore changes only through an explicit balanced administrator transaction;
transport receipt, order acceptance and fills do not synthesize a fee.

## `bunting.news.audience.v1`

News is immutable after commit and ordered by committed publication. Public
items are visible to every authenticated role. Participant and team items are
visible only to the exact authenticated binding; role-targeted items require
the exact role. Filtering occurs before FIX serialization.

## `bunting.tender.targeted-fixed-price.v1`

A tender pins one participant, instrument, side, positive lot quantity,
positive tick price and expiry in logical nanoseconds. Only the targeted
participant may accept or decline while its state is `open` and logical time is
strictly before expiry. The first committed decision wins; later decisions
reject. The MVC does not run an auction or infer a winner.

## `bunting.risk.scenario-limits.v1`

The run scenario pins maximum single-order quantity, maximum aggregate open
quantity and maximum absolute position in lots. Risk is checked before matching
and the participant FIX projection returns the same pinned limits. There is no
floating-point VaR or hidden risk score in this policy.

## `bunting.fine.explicit-cash.v1`

An instructor or administrator supplies a positive fine in minor currency
units, a participant, a currency and a non-empty audited reason. One balanced
journal transaction debits participant cash by the exact amount and credits
clearing by the same amount. Negative, zero, unbalanced or overflowing fines
reject before commit.

## `bunting.score.nlv-rank.v1`

At an explicit scoring command, each participant score equals exact net
liquidation value in the first instrument's settlement currency. Entries sort
by descending score, then ascending participant ID; rank is the one-based
position in that deterministic ordering. The frozen report records policy
version and logical generation time. No Sharpe adjustment is claimed.
