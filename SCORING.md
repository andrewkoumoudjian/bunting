# Scoring

The released policy is `bunting.score.nlv-rank.v1`. At settlement, the engine
values each participant's cash and inventory at the committed final market
state, applies committed fines and cashflows, sorts net liquidation value
descending, and uses participant ID as the deterministic tie-break.

Live scores are provisional theatre. The official result is the final score
report reproduced from the signed run archive:

```bash
bunting replay round.archive.json
bunting score round.archive.json
```

`bunting judge` verifies every supplied archive before writing
`leaderboard.json` and `leaderboard.html`; an event or final-state mismatch
fails the command and produces no valid settlement.
