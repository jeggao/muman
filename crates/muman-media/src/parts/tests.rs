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

#[test]
fn a_credit_s_names_are_parted_by_its_joiners_and_featured_after_feat() {
    let c = credit("Marlo Venn & Kiri Hoshino feat. Ada Quill, Glass Orchard");
    assert_eq!(
        c.names,
        ["Marlo Venn", "Kiri Hoshino", "Ada Quill", "Glass Orchard"]
    );
    assert_eq!(c.joiners, ["&", "feat.", ","]);
    assert_eq!(c.featured, Some(2));
    let one = credit("Marlo Venn");
    assert_eq!(
        (one.names, one.joiners, one.featured),
        (vec!["Marlo Venn"], vec![], None)
    );
    assert_eq!(
        credit(" & Marlo Venn & ").names,
        ["Marlo Venn"],
        "empty names add nothing"
    );
}

proptest::proptest! {
    #[test]
    fn every_name_is_read_from_the_credit_as_written(raw in "[A-Za-z ]{0,6}((&|, | feat\\. | x )[A-Za-z ]{0,6}){0,3}") {
        let c = credit(&raw);
        proptest::prop_assert_eq!(c.joiners.len(), c.names.len().saturating_sub(1));
        for name in &c.names {
            proptest::prop_assert!(raw.contains(name), "{raw:?}: {c:?}");
        }
    }
}

#[test]
fn an_album_closing_on_a_disc_is_its_name_and_that_disc() {
    assert_eq!(
        album("Harbor Nights (CD2)"),
        Album {
            name: "Harbor Nights",
            disc: Some(2)
        }
    );
    assert_eq!(album("Harbor Nights / CD 1").disc, Some(1));
    assert_eq!(
        album(" Harbor Nights "),
        Album {
            name: "Harbor Nights",
            disc: None
        }
    );
}

#[test]
fn a_date_is_its_year_month_and_day_where_it_names_ones_there_are() {
    assert_eq!(
        date("2011-03-04"),
        Some(Date {
            year: 2011,
            month: Some(3),
            day: Some(4)
        })
    );
    assert_eq!(date("2011").map(|d| (d.month, d.day)), Some((None, None)));
    for bad in [
        "2011-13",
        "2011-03-32",
        "11-03-04",
        "2011-3",
        "",
        "2011-03-04-05",
    ] {
        assert_eq!(date(bad), None, "{bad}");
    }
}
