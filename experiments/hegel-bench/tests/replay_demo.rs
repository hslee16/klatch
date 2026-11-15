//! What a failing property looks like and how it replays (bench #390, docs #395). The test fails on purpose, so it is
//! ignored; run it with `cargo test --test replay_demo -- --ignored`.
use hegel::generators as gs;
use hegel::TestCase;

fn buggy_sum(items: &[u32]) -> u32 {
    // a planted bug: it forgets every item after the first that is 7 or more
    let mut total = 0;
    for &i in items {
        if i >= 7 && total > 0 { break; }
        total += i;
    }
    total
}

#[hegel::test]
#[ignore = "fails on purpose, to show the report and the replay"]
fn the_sum_of_a_list_is_the_sum_of_its_items(tc: TestCase) {
    let items = tc.draw(gs::vecs(gs::integers::<u32>().max_value(20)));
    assert_eq!(buggy_sum(&items), items.iter().sum::<u32>());
}
