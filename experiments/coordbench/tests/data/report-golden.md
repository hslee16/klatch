# Coordination bench report

Generated TIME from NEW. 37 run records, 1 invalid (kept, not analysed).

## Conditions

| Condition | Value |
|---|---|
| milestones | s |
| agent models | fake |
| resolved model ids | n/a |
| Claude Code version | n/a |
| klatch commit | abc |
| klatch binary sha256 | n/a |
| arm text version | draft-1 |
| fixture sha256 | 0000000000000000000000000000000000000000000000000000000000000000 |
| temperature | default |
| caps | {"max_turns_per_agent": 30, "per_run_usd": 1.0, "suite_usd": 10.0, "wall_s": 100} |

## Comparison with OLD

No regressions flagged (30 comparisons).

All compared cells:

| Scenario | Arm | Metric | Previous median | Now | Change | N old/new | p Holm | Flag |
|---|---|---|---|---|---|---|---|---|
| onefile | B0 | completion | 0.67 | 0.67 | 0 | 6/6 | 1.0000 |  |
| onefile | B0 | units_completed | 2 | 2 | 0 | 6/6 | 1.0000 |  |
| onefile | B0 | cost_usd | 0.40 | 0.35 | -0.05 | 6/6 | 1.0000 |  |
| onefile | B0 | turns | 12.50 | 12.50 | 0 | 6/6 | 1.0000 |  |
| onefile | B0 | wall_s | 73 | 72.50 | -0.50 | 6/6 | 1.0000 |  |
| onefile | K2 | completion | 0.67 | 0.67 | 0 | 3/3 | 1.0000 |  |
| onefile | K2 | units_completed | 2 | 2 | 0 | 3/3 | 1.0000 |  |
| onefile | K2 | cost_usd | 0.42 | 0.37 | -0.05 | 3/3 | 1.0000 |  |
| onefile | K2 | turns | 14 | 14 | 0 | 3/3 | 1.0000 |  |
| onefile | K2 | wall_s | 72.50 | 72 | -0.50 | 3/3 | 1.0000 |  |
| trivial | B0 | completion | 0.57 | 0.57 | 0 | 7/7 | 1.0000 |  |
| trivial | B0 | units_completed | 2 | 2 | 0 | 7/7 | 1.0000 |  |
| trivial | B0 | cost_usd | 0.41 | 0.36 | -0.05 | 7/7 | 1.0000 |  |
| trivial | B0 | turns | 13 | 13 | 0 | 7/7 | 1.0000 |  |
| trivial | B0 | wall_s | 75.50 | 75 | -0.50 | 7/7 | 1.0000 |  |
| trivial | B1 | completion | 0.71 | 0.71 | 0 | 7/7 | 1.0000 |  |
| trivial | B1 | units_completed | 2 | 2 | 0 | 7/7 | 1.0000 |  |
| trivial | B1 | cost_usd | 0.46 | 0.41 | -0.05 | 7/7 | 1.0000 |  |
| trivial | B1 | turns | 16 | 16 | 0 | 7/7 | 1.0000 |  |
| trivial | B1 | wall_s | 82.50 | 82 | -0.50 | 7/7 | 1.0000 |  |
| trivial | K1 | completion | 0.67 | 0.67 | 0 | 6/6 | 1.0000 |  |
| trivial | K1 | units_completed | 2 | 2 | 0 | 6/6 | 1.0000 |  |
| trivial | K1 | cost_usd | 0.50 | 0.45 | -0.05 | 6/6 | 1.0000 |  |
| trivial | K1 | turns | 18.50 | 18.50 | 0 | 6/6 | 1.0000 |  |
| trivial | K1 | wall_s | 87 | 86.50 | -0.50 | 6/6 | 1.0000 |  |
| trivial | K2 | completion | 0.57 | 0.57 | 0 | 7/7 | 1.0000 |  |
| trivial | K2 | units_completed | 2 | 2 | 0 | 7/7 | 1.0000 |  |
| trivial | K2 | cost_usd | 0.56 | 0.51 | -0.05 | 7/7 | 1.0000 |  |
| trivial | K2 | turns | 22 | 22 | 0 | 7/7 | 1.0000 |  |
| trivial | K2 | wall_s | 96.50 | 96 | -0.50 | 7/7 | 1.0000 |  |

Tests made: 40 (primary-metric tests 10, Holm-corrected across the whole report, primary metric `units_completed` plus the completion rate). Bootstrap: 400 resamples, seed 20261122.

## Scenario onefile

**EXPLORATORY**: at least one arm has fewer than 5 runs. The tables are the result; no significance claim is made.

| Arm | N | invalid | Completed (all units) | Wilson 95% | Outcomes | Units median | Cost USD median | Turns median | Wall s median |
|---|---|---|---|---|---|---|---|---|---|
| B0 | 6 | 0 | 4 of 6 (0.67) | [0.30, 0.90] | ok 6 | 2 | 0.350 | 12.50 | 72.50 (0 censored) |
| K2 | 3 | 0 | 2 of 3 (0.67) | [0.21, 0.94] | ok 3 | 2 | 0.370 | 14 | 72 (0 censored) |

Per metric, median with quartiles, range, then mean and sd as a second line:

| Arm | Metric | N | median | q25 | q75 | min | max | mean | sd |
|---|---|---|---|---|---|---|---|---|---|
| B0 | units_completed | 6 | 2 | 1.250 | 2 | 1 | 2 | 1.667 | 0.516 |
| B0 | cost_usd | 6 | 0.350 | 0.325 | 0.375 | 0.300 | 0.400 | 0.350 | 0.037 |
| B0 | turns | 6 | 12.500 | 11.250 | 13.750 | 10 | 15 | 12.500 | 1.871 |
| B0 | wall_s | 6 | 72.500 | 66.250 | 78.750 | 60 | 85 | 72.500 | 9.354 |
| B0 | tokens | 6 | 1250 | 1125 | 1375 | 1000 | 1500 | 1250 | 187.083 |
| B0 | messages | 6 | 2.500 | 1.250 | 3.750 | 0 | 5 | 2.500 | 1.871 |
| B0 | git_conflicts | 6 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| B0 | coordination_context_tokens | 6 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| B0 | idle_turns | 6 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| B0 | human_interventions | 6 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| K2 | units_completed | 3 | 2 | 1.500 | 2 | 1 | 2 | 1.667 | 0.577 |
| K2 | cost_usd | 3 | 0.370 | 0.360 | 0.380 | 0.350 | 0.390 | 0.370 | 0.020 |
| K2 | turns | 3 | 14 | 13.500 | 14.500 | 13 | 15 | 14 | 1 |
| K2 | wall_s | 3 | 72 | 69.500 | 74.500 | 67 | 77 | 72 | 5 |
| K2 | tokens | 3 | 1100 | 1050 | 1150 | 1000 | 1200 | 1100 | 100 |
| K2 | messages | 3 | 3 | 2.500 | 3.500 | 2 | 4 | 3 | 1 |
| K2 | git_conflicts | 3 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| K2 | coordination_context_tokens | 3 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| K2 | idle_turns | 3 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| K2 | human_interventions | 3 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |

Tests, each K arm against each B arm. The shift is the first arm minus the second.

| Pair | Metric | Test | N | p | p Holm | Effect | Reading |
|---|---|---|---|---|---|---|---|
| K2 vs B0 | completion | Fisher exact | 3/6 | 1.0000 | 1.0000 | diff 0 [-0.52, 0.46], risk ratio 1 | EXPLORATORY, no claim |
| K2 vs B0 | units_completed | Mann-Whitney U (normal-ties) | 3/6 | 1.0000 | 1.0000 | shift 0 [-1, 1] | EXPLORATORY, no claim |
| K2 vs B0 | cost_usd | Mann-Whitney U (exact) | 3/6 | 0.5476 | n/a | shift 0.020 [-0.030, 0.070] | EXPLORATORY, no claim |
| K2 vs B0 | turns | Mann-Whitney U (normal-ties) | 3/6 | 0.2956 | n/a | shift 1.500 [-1, 4] | EXPLORATORY, no claim |
| K2 vs B0 | wall_s | Mann-Whitney U (exact) | 3/6 | 1.0000 | n/a | shift -0.500 [-13, 12]; p without censored 1.0000 | EXPLORATORY, no claim |
| K2 vs B0 | tokens | Mann-Whitney U (normal-ties) | 3/6 | 0.2956 | n/a | shift -150 [-400, 100] | EXPLORATORY, no claim |
| K2 vs B0 | messages | Mann-Whitney U (normal-ties) | 3/6 | 0.7937 | n/a | shift 0.500 [-2, 3] | EXPLORATORY, no claim |
| K2 vs B0 | coordination_context_tokens | Mann-Whitney U (normal-ties) | 3/6 | 1.0000 | n/a | shift 0 [0, 0] | EXPLORATORY, no claim |

Reading the tests: with 10 runs per arm, Fisher exact detects 0.9 against 0.2 with power 0.84 but 0.9 against 0.4 with only 0.47 (SPEC 6.1), and Mann-Whitney detects a shift of about 1.3 standard deviations with power 0.78. A result of no significant difference is not the same as no difference; effects under about one standard deviation are out of reach at affordable N. Means and standard deviations are a second line.

## Scenario trivial

| Arm | N | invalid | Completed (all units) | Wilson 95% | Outcomes | Units median | Cost USD median | Turns median | Wall s median |
|---|---|---|---|---|---|---|---|---|---|
| B0 | 7 | 0 | 4 of 7 (0.57) | [0.25, 0.84] | ok 7 | 2 | 0.360 | 13 | 75 (0 censored) |
| B1 | 7 | 0 | 5 of 7 (0.71) | [0.36, 0.92] | ok 6, timeout 1 | 2 | 0.410 | 16 | 82 (1 censored) |
| K1 | 6 | 1 | 4 of 6 (0.67) | [0.30, 0.90] | ok 6 | 2 | 0.450 | 18.50 | 86.50 (0 censored) |
| K2 | 7 | 0 | 4 of 7 (0.57) | [0.25, 0.84] | ok 7 | 2 | 0.510 | 22 | 96 (0 censored) |

Per metric, median with quartiles, range, then mean and sd as a second line:

| Arm | Metric | N | median | q25 | q75 | min | max | mean | sd |
|---|---|---|---|---|---|---|---|---|---|
| B0 | units_completed | 7 | 2 | 1 | 2 | 1 | 2 | 1.571 | 0.535 |
| B0 | cost_usd | 7 | 0.360 | 0.330 | 0.390 | 0.300 | 0.420 | 0.360 | 0.043 |
| B0 | turns | 7 | 13 | 11.500 | 14.500 | 10 | 16 | 13 | 2.160 |
| B0 | wall_s | 7 | 75 | 67.500 | 82.500 | 60 | 90 | 75 | 10.801 |
| B0 | tokens | 7 | 1300 | 1150 | 1450 | 1000 | 1600 | 1300 | 216.025 |
| B0 | messages | 7 | 3 | 1.500 | 4.500 | 0 | 6 | 3 | 2.160 |
| B0 | git_conflicts | 7 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| B0 | coordination_context_tokens | 7 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| B0 | idle_turns | 7 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| B0 | human_interventions | 7 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| B1 | units_completed | 7 | 2 | 1.500 | 2 | 1 | 2 | 1.714 | 0.488 |
| B1 | cost_usd | 7 | 0.410 | 0.380 | 0.440 | 0.350 | 0.470 | 0.410 | 0.043 |
| B1 | turns | 7 | 16 | 14.500 | 17.500 | 13 | 19 | 16 | 2.160 |
| B1 | wall_s | 7 | 82 | 74.500 | 89.500 | 67 | 97 | 82 | 10.801 |
| B1 | tokens | 7 | 1300 | 1150 | 1450 | 1000 | 1600 | 1300 | 216.025 |
| B1 | messages | 7 | 5 | 3.500 | 6.500 | 2 | 8 | 5 | 2.160 |
| B1 | git_conflicts | 7 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| B1 | coordination_context_tokens | 7 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| B1 | idle_turns | 7 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| B1 | human_interventions | 7 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| K1 | units_completed | 6 | 2 | 1.250 | 2 | 1 | 2 | 1.667 | 0.516 |
| K1 | cost_usd | 6 | 0.450 | 0.425 | 0.475 | 0.400 | 0.500 | 0.450 | 0.037 |
| K1 | turns | 6 | 18.500 | 17.250 | 19.750 | 16 | 21 | 18.500 | 1.871 |
| K1 | wall_s | 6 | 86.500 | 80.250 | 92.750 | 74 | 99 | 86.500 | 9.354 |
| K1 | tokens | 6 | 1250 | 1125 | 1375 | 1000 | 1500 | 1250 | 187.083 |
| K1 | messages | 6 | 6.500 | 5.250 | 7.750 | 4 | 9 | 6.500 | 1.871 |
| K1 | git_conflicts | 6 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| K1 | coordination_context_tokens | 6 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| K1 | idle_turns | 6 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| K1 | human_interventions | 6 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| K2 | units_completed | 7 | 2 | 1 | 2 | 1 | 2 | 1.571 | 0.535 |
| K2 | cost_usd | 7 | 0.510 | 0.480 | 0.540 | 0.450 | 0.570 | 0.510 | 0.043 |
| K2 | turns | 7 | 22 | 20.500 | 23.500 | 19 | 25 | 22 | 2.160 |
| K2 | wall_s | 7 | 96 | 88.500 | 103.500 | 81 | 111 | 96 | 10.801 |
| K2 | tokens | 7 | 1300 | 1150 | 1450 | 1000 | 1600 | 1300 | 216.025 |
| K2 | messages | 7 | 9 | 7.500 | 10.500 | 6 | 12 | 9 | 2.160 |
| K2 | git_conflicts | 7 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| K2 | coordination_context_tokens | 7 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| K2 | idle_turns | 7 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| K2 | human_interventions | 7 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |

Tests, each K arm against each B arm. The shift is the first arm minus the second.

| Pair | Metric | Test | N | p | p Holm | Effect | Reading |
|---|---|---|---|---|---|---|---|
| K1 vs B0 | completion | Fisher exact | 6/7 | 1.0000 | 1.0000 | diff 0.10 [-0.36, 0.49], risk ratio 1.17 | not significant |
| K1 vs B0 | units_completed | Mann-Whitney U (normal-ties) | 6/7 | 0.7998 | 1.0000 | shift 0 [-1, 1] | not significant |
| K1 vs B0 | cost_usd | Mann-Whitney U (normal-ties) | 6/7 | 0.0080 | n/a | shift 0.090 [0.030, 0.160] | no claim (not primary) |
| K1 vs B0 | turns | Mann-Whitney U (normal-ties) | 6/7 | 0.0042 | n/a | shift 5.500 [2.500, 9] | no claim (not primary) |
| K1 vs B0 | wall_s | Mann-Whitney U (exact) | 6/7 | 0.1375 | n/a | shift 11.500 [-3.500, 29]; p without censored 0.1375 | no claim (not primary) |
| K1 vs B0 | tokens | Mann-Whitney U (normal-ties) | 6/7 | 0.7188 | n/a | shift -50 [-350, 300] | no claim (not primary) |
| K1 vs B0 | messages | Mann-Whitney U (normal-ties) | 6/7 | 0.0217 | n/a | shift 3.500 [0.500, 7] | no claim (not primary) |
| K1 vs B0 | coordination_context_tokens | Mann-Whitney U (normal-ties) | 6/7 | 1.0000 | n/a | shift 0 [0, 0] | no claim (not primary) |
| K1 vs B1 | completion | Fisher exact | 6/7 | 1.0000 | 1.0000 | diff -0.05 [-0.47, 0.38], risk ratio 0.93 | not significant |
| K1 vs B1 | units_completed | Mann-Whitney U (normal-ties) | 6/7 | 0.9290 | 1.0000 | shift 0 [-1, 1] | not significant |
| K1 vs B1 | cost_usd | Mann-Whitney U (exact) | 6/7 | 0.1375 | n/a | shift 0.040 [-0.020, 0.110] | no claim (not primary) |
| K1 vs B1 | turns | Mann-Whitney U (normal-ties) | 6/7 | 0.0726 | n/a | shift 2.500 [-0.500, 6] | no claim (not primary) |
| K1 vs B1 | wall_s | Mann-Whitney U (exact) | 6/7 | 0.4452 | n/a | shift 4.500 [-10.500, 22]; p without censored 0.3095 | no claim (not primary) |
| K1 vs B1 | tokens | Mann-Whitney U (normal-ties) | 6/7 | 0.7188 | n/a | shift -50 [-350, 300] | no claim (not primary) |
| K1 vs B1 | messages | Mann-Whitney U (normal-ties) | 6/7 | 0.2498 | n/a | shift 1.500 [-1.500, 5] | no claim (not primary) |
| K1 vs B1 | coordination_context_tokens | Mann-Whitney U (normal-ties) | 6/7 | 1.0000 | n/a | shift 0 [0, 0] | no claim (not primary) |
| K2 vs B0 | completion | Fisher exact | 7/7 | 1.0000 | 1.0000 | diff 0 [-0.42, 0.42], risk ratio 1 | not significant |
| K2 vs B0 | units_completed | Mann-Whitney U (normal-ties) | 7/7 | 1.0000 | 1.0000 | shift 0 [-1, 1] | not significant |
| K2 vs B0 | cost_usd | Mann-Whitney U (exact) | 7/7 | 0.0006 | n/a | shift 0.150 [0.090, 0.210] | no claim (not primary) |
| K2 vs B0 | turns | Mann-Whitney U (exact) | 7/7 | 0.0006 | n/a | shift 9 [6, 12] | no claim (not primary) |
| K2 vs B0 | wall_s | Mann-Whitney U (exact) | 7/7 | 0.0041 | n/a | shift 21 [6, 36]; p without censored 0.0041 | no claim (not primary) |
| K2 vs B0 | tokens | Mann-Whitney U (normal-ties) | 7/7 | 1.0000 | n/a | shift 0 [-300, 300] | no claim (not primary) |
| K2 vs B0 | messages | Mann-Whitney U (normal-ties) | 7/7 | 0.0026 | n/a | shift 6 [3, 9] | no claim (not primary) |
| K2 vs B0 | coordination_context_tokens | Mann-Whitney U (normal-ties) | 7/7 | 1.0000 | n/a | shift 0 [0, 0] | no claim (not primary) |
| K2 vs B1 | completion | Fisher exact | 7/7 | 1.0000 | 1.0000 | diff -0.14 [-0.52, 0.30], risk ratio 0.80 | not significant |
| K2 vs B1 | units_completed | Mann-Whitney U (normal-ties) | 7/7 | 0.6450 | 1.0000 | shift 0 [-1, 1] | not significant |
| K2 vs B1 | cost_usd | Mann-Whitney U (normal-ties) | 7/7 | 0.0048 | n/a | shift 0.100 [0.040, 0.160] | no claim (not primary) |
| K2 vs B1 | turns | Mann-Whitney U (normal-ties) | 7/7 | 0.0026 | n/a | shift 6 [3, 9] | no claim (not primary) |
| K2 vs B1 | wall_s | Mann-Whitney U (exact) | 7/7 | 0.0728 | n/a | shift 14 [-1, 29]; p without censored 0.0513 | no claim (not primary) |
| K2 vs B1 | tokens | Mann-Whitney U (normal-ties) | 7/7 | 1.0000 | n/a | shift 0 [-300, 300] | no claim (not primary) |
| K2 vs B1 | messages | Mann-Whitney U (normal-ties) | 7/7 | 0.0124 | n/a | shift 4 [1, 7] | no claim (not primary) |
| K2 vs B1 | coordination_context_tokens | Mann-Whitney U (normal-ties) | 7/7 | 1.0000 | n/a | shift 0 [0, 0] | no claim (not primary) |

Reading the tests: with 10 runs per arm, Fisher exact detects 0.9 against 0.2 with power 0.84 but 0.9 against 0.4 with only 0.47 (SPEC 6.1), and Mann-Whitney detects a shift of about 1.3 standard deviations with power 0.78. A result of no significant difference is not the same as no difference; effects under about one standard deviation are out of reach at affordable N. Means and standard deviations are a second line.

=====COMPARISON-ONLY=====
# Coordination bench report

Generated TIME from NEW. 37 run records, 1 invalid (kept, not analysed).

## Comparison with OLD

No regressions flagged (30 comparisons).

All compared cells:

| Scenario | Arm | Metric | Previous median | Now | Change | N old/new | p Holm | Flag |
|---|---|---|---|---|---|---|---|---|
| onefile | B0 | completion | 0.67 | 0.67 | 0 | 6/6 | 1.0000 |  |
| onefile | B0 | units_completed | 2 | 2 | 0 | 6/6 | 1.0000 |  |
| onefile | B0 | cost_usd | 0.40 | 0.35 | -0.05 | 6/6 | 1.0000 |  |
| onefile | B0 | turns | 12.50 | 12.50 | 0 | 6/6 | 1.0000 |  |
| onefile | B0 | wall_s | 73 | 72.50 | -0.50 | 6/6 | 1.0000 |  |
| onefile | K2 | completion | 0.67 | 0.67 | 0 | 3/3 | 1.0000 |  |
| onefile | K2 | units_completed | 2 | 2 | 0 | 3/3 | 1.0000 |  |
| onefile | K2 | cost_usd | 0.42 | 0.37 | -0.05 | 3/3 | 1.0000 |  |
| onefile | K2 | turns | 14 | 14 | 0 | 3/3 | 1.0000 |  |
| onefile | K2 | wall_s | 72.50 | 72 | -0.50 | 3/3 | 1.0000 |  |
| trivial | B0 | completion | 0.57 | 0.57 | 0 | 7/7 | 1.0000 |  |
| trivial | B0 | units_completed | 2 | 2 | 0 | 7/7 | 1.0000 |  |
| trivial | B0 | cost_usd | 0.41 | 0.36 | -0.05 | 7/7 | 1.0000 |  |
| trivial | B0 | turns | 13 | 13 | 0 | 7/7 | 1.0000 |  |
| trivial | B0 | wall_s | 75.50 | 75 | -0.50 | 7/7 | 1.0000 |  |
| trivial | B1 | completion | 0.71 | 0.71 | 0 | 7/7 | 1.0000 |  |
| trivial | B1 | units_completed | 2 | 2 | 0 | 7/7 | 1.0000 |  |
| trivial | B1 | cost_usd | 0.46 | 0.41 | -0.05 | 7/7 | 1.0000 |  |
| trivial | B1 | turns | 16 | 16 | 0 | 7/7 | 1.0000 |  |
| trivial | B1 | wall_s | 82.50 | 82 | -0.50 | 7/7 | 1.0000 |  |
| trivial | K1 | completion | 0.67 | 0.67 | 0 | 6/6 | 1.0000 |  |
| trivial | K1 | units_completed | 2 | 2 | 0 | 6/6 | 1.0000 |  |
| trivial | K1 | cost_usd | 0.50 | 0.45 | -0.05 | 6/6 | 1.0000 |  |
| trivial | K1 | turns | 18.50 | 18.50 | 0 | 6/6 | 1.0000 |  |
| trivial | K1 | wall_s | 87 | 86.50 | -0.50 | 6/6 | 1.0000 |  |
| trivial | K2 | completion | 0.57 | 0.57 | 0 | 7/7 | 1.0000 |  |
| trivial | K2 | units_completed | 2 | 2 | 0 | 7/7 | 1.0000 |  |
| trivial | K2 | cost_usd | 0.56 | 0.51 | -0.05 | 7/7 | 1.0000 |  |
| trivial | K2 | turns | 22 | 22 | 0 | 7/7 | 1.0000 |  |
| trivial | K2 | wall_s | 96.50 | 96 | -0.50 | 7/7 | 1.0000 |  |
