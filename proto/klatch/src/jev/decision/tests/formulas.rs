use super::*;

// ---- the documented formulas ----

#[test]
fn choice_confidence_is_the_documented_formula() {
    near(choice_confidence(&[0.6, 0.3, 0.1]), 0.4);
    near(choice_confidence(&[0.6, 0.2, 0.2]), 0.4); // only the top probability counts
    near(choice_confidence(&[0.75, 0.25]), 0.5);
    near(choice_confidence(&[1.0, 0.0, 0.0]), 1.0);
    near(choice_confidence(&[1.0 / 3.0; 3]), 0.0);
    near(choice_confidence(&[1.0]), 1.0);
}

#[test]
fn the_table_of_confidence_against_top_probability_holds() {
    // confidence.md: confidence 0.5 is a top probability of 0.75, 0.67 and 0.55 for 2, 3 and 10 options
    near(choice_confidence(&[0.75, 0.25]), 0.5);
    assert!((choice_confidence(&[0.67, 0.165, 0.165]) - 0.505).abs() < 0.01);
    let ten: Vec<f64> = std::iter::once(0.55)
        .chain(std::iter::repeat_n(0.45 / 9.0, 9))
        .collect();
    near(choice_confidence(&ten), 0.5);
}

#[test]
fn a_noul_has_no_confidence_so_the_distance_from_one_half_stands_in() {
    near(noul_confidence(0.5), 0.0);
    near(noul_confidence(0.0), 1.0);
    near(noul_confidence(1.0), 1.0);
    near(noul_confidence(0.85), 0.7);
}

#[test]
fn score_confidence_counts_the_distance_between_levels() {
    // confidence.md: (0, 0.5, 0.5) is 0.25 and (0.5, 0, 0.5) is 0 for three levels; (0, 0.57, 0.43) is about 0.35
    near(score_confidence(&[0.0, 0.5, 0.5]), 0.25);
    near(score_confidence(&[0.5, 0.0, 0.5]), 0.0);
    assert!((score_confidence(&[0.0, 0.57, 0.43]) - 0.355).abs() < 0.01);
    near(score_confidence(&[0.0, 0.0, 1.0]), 1.0);
}

#[test]
fn the_top_to_second_ratio_tells_a_close_runner_up() {
    let a = ans("a", &[("a", 0.6), ("b", 0.3), ("c", 0.1)], 0.4);
    near(top_to_second_ratio(&a).unwrap(), 2.0);
    let sure = ans("a", &[("a", 1.0), ("b", 0.0)], 1.0);
    assert!(top_to_second_ratio(&sure).unwrap().is_infinite());
    let lone = ans("a", &[("a", 1.0)], 1.0);
    assert_eq!(top_to_second_ratio(&lone), None);
}

fn severity() -> ScoreAnswer {
    // the confidence-routing.md example: most of the mass on the top level, some on a low one
    ScoreAnswer {
        score: 3.19,
        confidence: 0.34,
        probabilities: vec![0.01, 0.14, 0.07, 0.19, 0.59],
        legend: vec![],
    }
}

#[test]
fn a_score_is_settled_only_at_or_above_the_confidence_floor_and_the_mass_of_levels_adds_up() {
    let a = severity();
    assert!(
        !a.is_settled(0.5),
        "confidence 0.34 is below the documented 0.5 floor"
    );
    assert!(a.is_settled(0.34));
    near(a.level_mass(3..=4), 0.78);
    near(a.level_mass(0..=0), 0.01);
    near(a.level_mass(4..=9), 0.59); // levels past the scale add nothing
    near(a.level_mass(5..=9), 0.0);
}

// ---- noul screen ----

#[test]
fn a_noul_is_a_clear_yes_a_clear_no_or_unsure() {
    let t = NoulThresholds::default();
    assert_eq!((t.yes_above(), t.no_below()), (0.7, 0.3));
    assert_eq!(t.band(0.98), Band::Yes);
    assert_eq!(t.band(0.71), Band::Yes);
    assert_eq!(t.band(0.7), Band::Unsure, "above 0.7, not at it");
    assert_eq!(t.band(0.44), Band::Unsure);
    assert_eq!(t.band(0.3), Band::Unsure, "below 0.3, not at it");
    assert_eq!(t.band(0.29), Band::No);
    assert_eq!(t.band(0.02), Band::No);
    assert_eq!(t.band(f64::NAN), Band::Unsure, "no number is not a yes");
}

#[test]
fn noul_thresholds_are_validated() {
    assert!(NoulThresholds::new(0.9, 0.1).is_ok());
    assert!(
        NoulThresholds::new(0.5, 0.5).is_err(),
        "the unsure band cannot vanish"
    );
    assert!(NoulThresholds::new(0.3, 0.7).is_err());
    assert!(NoulThresholds::new(1.1, 0.1).is_err());
    assert!(NoulThresholds::new(0.9, -0.1).is_err());
    assert!(NoulThresholds::new(f64::NAN, 0.1).is_err());
    assert!(NoulThresholds::new(0.9, f64::NAN).is_err());
}
