use super::*;

fn kinds(t: &Title) -> Vec<(&str, Kind)> {
    t.decorations
        .iter()
        .map(|d| (d.text.as_str(), d.kind))
        .collect()
}

#[test]
fn a_title_s_decorations_are_read_as_what_each_says() {
    let t = title(
        "Lantern Weather (feat. Kiri Hoshino) [Live at the Pier] (Official Video) - Radio Edit",
    );
    assert_eq!(t.core, "Lantern Weather");
    assert_eq!(
        kinds(&t),
        [
            ("feat. Kiri Hoshino", Kind::Featuring),
            ("Live at the Pier", Kind::Version("live version")),
            ("Official Video", Kind::Packaging),
            ("Radio Edit", Kind::Soft),
        ]
    );
    assert_eq!(t.decorations[1].brackets, Some(('[', ']')));
    assert_eq!(t.decorations[3].brackets, None, "after a spaced dash");
    let alone = title("【Night Ferry】(MV)");
    assert_eq!(alone.core, "Night Ferry", "the first that is no packaging");
    assert_eq!(
        kinds(&title("夜の船 〜Harbor Mix〜")),
        [("Harbor Mix", Kind::Soft)]
    );
}

/// Whether `part`'s characters stand in `raw` in its order.
fn within(part: &str, raw: &str) -> bool {
    let mut rest = raw.chars();
    part.chars().all(|c| rest.by_ref().any(|r| r == c))
}

proptest::proptest! {
    #[test]
    fn every_part_is_read_from_the_title_as_written(
        raw in "[A-Za-z ]{0,8}([(\\[【〜]?[A-Za-z. ]{0,8}[)\\]】〜]?)( - [A-Za-z ]{0,8})?",
    ) {
        let t = title(&raw);
        proptest::prop_assert!(within(&t.core, &raw), "{raw:?}: {t:?}");
        for d in &t.decorations {
            proptest::prop_assert!(raw.contains(&d.text), "{raw:?}: {d:?}");
        }
    }
}
