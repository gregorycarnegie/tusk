//! Changes to people already in Net2, without the browser: the year change
//! that moves every year group up, and the record that marks someone a leaver.
//! `manage_client` makes the requests.
use std::collections::{BTreeMap, HashSet};

use serde_json::Value;

/// A year group's department: "Y12", "Year 12" or "year12", as its prefix
/// and number. Anything else, "Yellow" or "Year 12 tutors", is not one.
fn year_of(name: &str) -> Option<(&str, u8)> {
    let at = name.find(|c: char| c.is_ascii_digit())?;
    let (prefix, digits) = name.split_at(at);
    let word = prefix.trim().to_lowercase();
    if !matches!(word.as_str(), "y" | "year") || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((prefix, digits.parse().ok()?))
}

/// One request of the year change.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Rename { id: i32, from: String, to: String },
    Create(String),
}

impl Step {
    pub fn describe(&self) -> String {
        match self {
            Self::Rename { from, to, .. } => format!("{from} becomes {to}"),
            Self::Create(name) => format!("a new, empty {name}"),
        }
    }
}

/// Moves every year group up by renaming its department, so each student
/// keeps their department and nobody is moved one at a time. The top year
/// becomes `leavers` and a new, empty lowest year is made.
///
/// Renaming runs from the top down, so no name is ever taken twice. The
/// `years` are given, never the ones found: after a stopped run, the highest
/// left is the year below the top, and the lowest has not been made yet. A step whose new name is already taken counts as done, so
/// planning again after a stop finishes the change instead of moving anyone
/// up twice. That is also why an existing `leavers` department means the
/// change is already done this year.
pub fn promotion(
    departments: &[(i32, String)],
    years: std::ops::RangeInclusive<u8>,
    leavers: &str,
) -> Vec<Step> {
    let (lowest, top) = (*years.start(), *years.end());
    let mut taken: HashSet<String> = departments.iter().map(|(_, n)| n.to_lowercase()).collect();
    let found: BTreeMap<u8, (i32, &str, &String)> = departments
        .iter()
        .filter_map(|(id, name)| year_of(name).map(|(prefix, n)| (n, (*id, prefix, name))))
        .filter(|(n, _)| years.contains(n))
        .collect();
    // The new year is named like the others.
    let Some(&(_, prefix, _)) = found.values().next() else {
        return Vec::new();
    };
    let mut steps = Vec::new();
    for n in years.rev() {
        let Some(&(id, prefix, from)) = found.get(&n) else {
            continue;
        };
        let to = if n == top {
            leavers.to_string()
        } else {
            format!("{prefix}{}", n + 1)
        };
        if taken.insert(to.to_lowercase()) {
            taken.remove(&from.to_lowercase());
            steps.push(Step::Rename {
                id,
                from: from.clone(),
                to,
            });
        }
    }
    let first = format!("{prefix}{lowest}");
    if !taken.contains(&first.to_lowercase()) {
        steps.push(Step::Create(first));
    }
    steps
}

/// The user as Net2 sent it, with only the expiry changed to the end of
/// `last_day` (`YYYY-MM-DD`), the time Add people uses. `PUT /users/{id}`
/// replaces the whole record and clears what it is not sent, so everything
/// else goes back exactly as it came.
///
/// `Ok(None)` when their access already ends sooner: marking someone a
/// leaver must never give back access that has ended.
pub fn with_expiry(body: &str, expected: i32, last_day: &str) -> Result<Option<Value>, String> {
    let shaped = last_day.len() == 10
        && last_day.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            _ => b.is_ascii_digit(),
        });
    if !shaped {
        return Err("Choose the last day they can get in.".into());
    }
    let current = crate::net2::read_user(body, expected)?;
    if current
        .expires
        .is_some_and(|date| date.as_str() <= last_day)
    {
        return Ok(None);
    }
    let mut user: Value = serde_json::from_str(body).map_err(|_| UNEXPECTED.to_string())?;
    let record = user.as_object_mut().ok_or(UNEXPECTED)?;
    // Net2 documents camelCase but has sent PascalCase: keep its spelling,
    // and never send both.
    let key = if record.contains_key("Id") {
        "ExpiryDate"
    } else {
        "expiryDate"
    };
    record.insert(key.into(), format!("{last_day}T23:59:00").into());
    Ok(Some(user))
}

const UNEXPECTED: &str = "Net2 sent a reply Tusk does not understand.";

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use rstest::rstest;
    use serde_json::json;

    fn school(names: &[&str]) -> Vec<(i32, String)> {
        names
            .iter()
            .enumerate()
            .map(|(i, name)| (i as i32 + 1, name.to_string()))
            .collect()
    }

    /// What Net2's departments are after the steps run.
    fn apply(departments: &mut Vec<(i32, String)>, steps: &[Step]) {
        for step in steps {
            match step {
                Step::Rename { id, to, .. } => {
                    departments.iter_mut().find(|(d, _)| d == id).unwrap().1 = to.clone();
                }
                Step::Create(name) => {
                    departments.push((100 + departments.len() as i32, name.clone()))
                }
            }
        }
    }

    fn names(departments: &[(i32, String)]) -> Vec<(i32, &str)> {
        let mut names: Vec<_> = departments
            .iter()
            .map(|(id, n)| (*id, n.as_str()))
            .collect();
        names.sort();
        names
    }

    const SCHOOL: [&str; 9] = [
        "Staff", "Y7", "Y8", "Y9", "Y10", "Y11", "Y12", "Y13", "Leavers",
    ];

    #[test]
    fn every_year_moves_up_and_a_new_lowest_year_is_made() {
        let steps = promotion(&school(&SCHOOL), 7..=13, "Leavers 2027");
        let described: Vec<_> = steps.iter().map(Step::describe).collect();
        assert_eq!(
            described,
            [
                "Y13 becomes Leavers 2027",
                "Y12 becomes Y13",
                "Y11 becomes Y12",
                "Y10 becomes Y11",
                "Y9 becomes Y10",
                "Y8 becomes Y9",
                "Y7 becomes Y8",
                "a new, empty Y7",
            ]
        );
    }

    /// Whatever step a run stops after, planning again finishes it with
    /// everyone exactly one year up, and a finished change plans nothing.
    #[test]
    fn a_stopped_year_change_finishes_without_moving_anyone_twice() {
        let fresh = school(&SCHOOL);
        let steps = promotion(&fresh, 7..=13, "Leavers 2027");
        let mut whole = fresh.clone();
        apply(&mut whole, &steps);
        for done in 0..=steps.len() {
            let mut departments = fresh.clone();
            apply(&mut departments, &steps[..done]);
            let rest = promotion(&departments, 7..=13, "Leavers 2027");
            apply(&mut departments, &rest);
            assert_eq!(
                names(&departments),
                names(&whole),
                "stopped after {done} steps"
            );
            assert_eq!(
                promotion(&departments, 7..=13, "Leavers 2027"),
                [],
                "after {done} steps"
            );
        }
    }

    #[rstest]
    #[case::year_and_a_space(&["Year 10", "Year 11"], 10..=11, &["Year 11 becomes Leavers", "Year 10 becomes Year 11", "a new, empty Year 10"])]
    #[case::lower_case(&["year7", "year8"], 7..=8, &["year8 becomes Leavers", "year7 becomes year8", "a new, empty year7"])]
    #[case::not_year_groups(&["Yellow", "Year 12 tutors", "Y12a", "Staff"], 7..=13, &[])]
    #[case::years_outside_the_range_are_left_alone(&["Y6", "Y11", "Y12"], 7..=11, &["Y11 becomes Leavers", "a new, empty Y7"])]
    #[case::a_missing_middle_year_is_skipped(&["Y7", "Y9"], 7..=9, &["Y9 becomes Leavers", "Y7 becomes Y8", "a new, empty Y7"])]
    fn only_year_groups_in_the_range_move(
        #[case] departments: &[&str],
        #[case] years: std::ops::RangeInclusive<u8>,
        #[case] expected: &[&str],
    ) {
        let steps = promotion(&school(departments), years, "Leavers");
        let described: Vec<_> = steps.iter().map(Step::describe).collect();
        assert_eq!(described, expected);
    }

    /// The safe failure: if someone made this year's leavers department
    /// already, nothing moves rather than a year group moving twice.
    #[test]
    fn an_existing_leavers_department_counts_as_this_years_change_done() {
        let departments = school(&["Y12", "Y13", "leavers 2027"]);
        assert_eq!(promotion(&departments, 12..=13, "Leavers 2027"), []);
    }

    #[test]
    fn a_leaver_keeps_everything_but_the_expiry() {
        let user = json!({
            "id": 8, "firstName": "Jane", "lastName": "Doe", "pin": "0123", "hasImage": true,
            "expiryDate": "0001-01-01T00:00:00", "customFields": [{"id": 14, "value": "A1"}],
            "doorAccessPermissionSet": {"accessLevels": [1]},
        });
        let mut expected = user.clone();
        expected["expiryDate"] = "2027-07-23T23:59:00".into();
        assert_eq!(
            with_expiry(&user.to_string(), 8, "2027-07-23"),
            Ok(Some(expected))
        );
    }

    #[test]
    fn a_pascal_case_record_keeps_its_spelling() {
        let user = json!({"Id": 8, "FirstName": "Jane", "HasImage": false}).to_string();
        let marked = with_expiry(&user, 8, "2027-07-23").unwrap();
        let expected = json!({"Id": 8, "FirstName": "Jane", "HasImage": false, "ExpiryDate": "2027-07-23T23:59:00"});
        assert_eq!(marked, Some(expected));
    }

    #[rstest]
    #[case::ended_already("2020-07-31T23:59:00", None)]
    #[case::ends_that_day("2027-07-23T23:59:00", None)]
    #[case::ends_later("2030-07-31T23:59:00", Some("2027-07-23T23:59:00"))]
    fn a_leaver_never_gets_back_access_that_has_ended(
        #[case] expiry: &str,
        #[case] sent: Option<&str>,
    ) {
        let user = json!({"id": 8, "hasImage": false, "expiryDate": expiry}).to_string();
        let marked = with_expiry(&user, 8, "2027-07-23").unwrap();
        assert_eq!(
            marked.map(|user| user["expiryDate"].clone()),
            sent.map(Value::from)
        );
    }

    #[rstest]
    #[case::someone_else(r#"{"id": 9, "hasImage": false}"#, "2027-07-23")]
    #[case::not_a_record("[]", "2027-07-23")]
    #[case::no_date(r#"{"id": 8, "hasImage": false}"#, "")]
    #[case::not_a_date(r#"{"id": 8, "hasImage": false}"#, "23/07/2027")]
    fn a_leaver_is_never_written_from_a_doubtful_record_or_date(
        #[case] body: &str,
        #[case] day: &str,
    ) {
        assert!(with_expiry(body, 8, day).is_err());
    }
}
