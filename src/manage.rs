//! Changes to people already in Net2, without the browser: the year change
//! that moves every year group up, and the record that marks someone a leaver.
//! `manage_client` makes the requests.
use std::collections::BTreeMap;

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
/// Each rename moves the vacant year down one place. Requiring a complete
/// range at the start lets a resumed run identify that vacancy from Net2
/// alone, including when departments use different spellings of "Year".
pub fn promotion(
    departments: &[(i32, String)],
    years: std::ops::RangeInclusive<u8>,
    leavers: &str,
) -> Result<Vec<Step>, String> {
    let (lowest, top) = (*years.start(), *years.end());
    if year_of(leavers).is_some() {
        return Err("Choose a leavers name that is distinct from a year group.".into());
    }
    let mut found = BTreeMap::new();
    for (id, name) in departments {
        if let Some((prefix, year)) = year_of(name)
            && years.contains(&year)
            && found.insert(year, (*id, prefix, name)).is_some()
        {
            return Err(format!(
                "Net2 has more than one department for year {year}."
            ));
        }
    }
    if found.is_empty() {
        return Ok(Vec::new());
    }
    let leavers_exists = departments
        .iter()
        .any(|(_, name)| name.eq_ignore_ascii_case(leavers));
    let gaps: Vec<_> = years.filter(|year| !found.contains_key(year)).collect();
    let mut steps = Vec::new();
    let vacant = match (leavers_exists, gaps.as_slice()) {
        (true, []) => return Ok(steps),
        (true, &[year]) => year,
        (false, []) => {
            let (id, _, from) = found[&top];
            steps.push(Step::Rename {
                id,
                from: from.clone(),
                to: leavers.into(),
            });
            top
        }
        _ => {
            return Err(format!(
                "Missing year departments {gaps:?}. Check the selected range and leavers name in Net2."
            ));
        }
    };
    for n in (lowest..vacant).rev() {
        let (id, prefix, from) = found[&n];
        steps.push(Step::Rename {
            id,
            from: from.clone(),
            to: format!("{prefix}{}", n + 1),
        });
    }
    let (_, prefix, _) = found.values().next().unwrap();
    steps.push(Step::Create(format!("{prefix}{lowest}")));
    Ok(steps)
}

/// The user as Net2 sent it, with only the expiry changed to the end of
/// `last_day` (`YYYY-MM-DD`), the time Add people uses. `PUT /users/{id}`
/// replaces the whole record and clears what it is not sent, so everything
/// else goes back exactly as it came.
///
/// `Ok(None)` when their access already ends sooner: marking someone a
/// leaver must never give back access that has ended.
pub fn with_expiry(body: &str, expected: i32, last_day: &str) -> Result<Option<Value>, String> {
    if crate::people::read_date(last_day)?.as_deref() != Some(last_day) {
        return Err("Choose the last day they can get in.".into());
    }
    let current = crate::net2::read_user(body, expected)?;
    if current
        .expires
        .is_some_and(|date| date.as_str() <= last_day)
    {
        return Ok(None);
    }
    expiry_record(body, Some(last_day)).map(Some)
}

/// Explicitly change or clear an existing expiry, preserving the other fields.
pub fn undo_expiry(
    body: &str,
    expected: i32,
    last_day: Option<&str>,
) -> Result<Option<Value>, String> {
    if let Some(day) = last_day
        && crate::people::read_date(day)?.as_deref() != Some(day)
    {
        return Err("Choose a valid expiry date.".into());
    }
    if crate::net2::read_user(body, expected)?.expires.is_none() {
        return Ok(None);
    }
    expiry_record(body, last_day).map(Some)
}

fn expiry_record(body: &str, last_day: Option<&str>) -> Result<Value, String> {
    let mut user: Value = serde_json::from_str(body).map_err(|_| UNEXPECTED)?;
    let record = user.as_object_mut().ok_or(UNEXPECTED)?;
    let mut keys = record
        .keys()
        .filter(|key| key.eq_ignore_ascii_case("expiryDate"));
    let key = keys.next().cloned();
    if keys.next().is_some() {
        return Err(UNEXPECTED.into());
    }
    let key = key.unwrap_or_else(|| {
        if record.contains_key("Id") {
            "ExpiryDate"
        } else {
            "expiryDate"
        }
        .into()
    });
    let expiry = last_day.map_or(Value::Null, |date| format!("{date}T23:59:00").into());
    record.insert(key, expiry);
    Ok(user)
}

const UNEXPECTED: &str = "Net2 sent a reply Tusk does not understand.";

/// The permission set with `level` as the person's only access level (`set`),
/// or without it, keeping their individual door permissions. Net2 gives each
/// person one access level: given several, it keeps the first.
pub fn with_level(body: &str, level: i32, set: bool) -> Result<Option<Value>, String> {
    let mut permissions: Value = serde_json::from_str(body).map_err(|_| UNEXPECTED)?;
    let record = permissions.as_object_mut().ok_or(UNEXPECTED)?;
    let (levels, individual) = if record.contains_key("AccessLevels") {
        ("AccessLevels", "IndividualPermissions")
    } else {
        ("accessLevels", "individualPermissions")
    };
    if !record.get(individual).is_some_and(Value::is_array)
        || !record.get(levels).is_some_and(Value::is_array)
    {
        return Err(UNEXPECTED.into());
    }
    let levels = record.get_mut(levels).unwrap().as_array_mut().unwrap();
    if levels
        .iter()
        .any(|id| id.as_i64().and_then(|id| i32::try_from(id).ok()).is_none())
    {
        return Err(UNEXPECTED.into());
    }
    let level = Value::from(level);
    let done = if set {
        *levels == [level.clone()]
    } else {
        !levels.contains(&level)
    };
    if done {
        return Ok(None);
    }
    if set {
        *levels = vec![level];
    } else {
        levels.retain(|id| *id != level);
    }
    Ok(Some(permissions))
}

/// Whether Net2's GET /temporaryAccess/users/{id} reply says the person has
/// temporary access. The shape is unverified: Net2 names the answer
/// `UserHasTemporaryAccess`, so a bare bool or that one field is accepted.
pub fn has_temporary_access(body: &str) -> Result<bool, String> {
    let value: Value = serde_json::from_str(body).map_err(|_| UNEXPECTED)?;
    value
        .as_bool()
        .or_else(|| crate::net2::field(&value, "userHasTemporaryAccess")?.as_bool())
        .ok_or_else(|| {
            format!(
                "{UNEXPECTED} It said: {}",
                body.chars().take(120).collect::<String>()
            )
        })
}

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
        let steps = promotion(&school(&SCHOOL), 7..=13, "Leavers 2027").unwrap();
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
    #[rstest]
    #[case::same_prefix(&SCHOOL)]
    #[case::mixed_prefixes(&["Y7", "Year 8", "Y9", "Year 10", "Y11", "Year 12", "Y13"])]
    fn a_stopped_year_change_finishes_without_moving_anyone_twice(#[case] names_before: &[&str]) {
        let fresh = school(names_before);
        let steps = promotion(&fresh, 7..=13, "Leavers 2027").unwrap();
        let mut whole = fresh.clone();
        apply(&mut whole, &steps);
        for done in 0..=steps.len() {
            let mut departments = fresh.clone();
            apply(&mut departments, &steps[..done]);
            let rest = promotion(&departments, 7..=13, "Leavers 2027").unwrap();
            apply(&mut departments, &rest);
            assert_eq!(
                names(&departments),
                names(&whole),
                "stopped after {done} steps"
            );
            assert_eq!(
                promotion(&departments, 7..=13, "Leavers 2027").unwrap(),
                [],
                "after {done} steps"
            );
        }
    }

    #[rstest]
    #[case::year_and_a_space(&["Year 10", "Year 11"], 10..=11, &["Year 11 becomes Leavers", "Year 10 becomes Year 11", "a new, empty Year 10"])]
    #[case::lower_case(&["year7", "year8"], 7..=8, &["year8 becomes Leavers", "year7 becomes year8", "a new, empty year7"])]
    #[case::not_year_groups(&["Yellow", "Year 12 tutors", "Y12a", "Staff"], 7..=13, &[])]
    #[case::years_outside_the_range_are_left_alone(&["Y6", "Y10", "Y11", "Y12"], 10..=11, &["Y11 becomes Leavers", "Y10 becomes Y11", "a new, empty Y10"])]
    fn only_year_groups_in_the_range_move(
        #[case] departments: &[&str],
        #[case] years: std::ops::RangeInclusive<u8>,
        #[case] expected: &[&str],
    ) {
        let steps = promotion(&school(departments), years, "Leavers").unwrap();
        let described: Vec<_> = steps.iter().map(Step::describe).collect();
        assert_eq!(described, expected);
    }

    /// The safe failure: if someone made this year's leavers department
    /// already, nothing moves rather than a year group moving twice.
    #[test]
    fn an_existing_leavers_department_counts_as_this_years_change_done() {
        let departments = school(&["Y12", "Y13", "leavers 2027"]);
        assert_eq!(
            promotion(&departments, 12..=13, "Leavers 2027").unwrap(),
            []
        );
    }

    #[rstest]
    #[case::gap(&["Y7", "Y9"])]
    #[case::duplicate_year(&["Y7", "Year 7", "Y8", "Y9"])]
    fn ambiguous_year_groups_are_refused(#[case] departments: &[&str]) {
        assert!(promotion(&school(departments), 7..=9, "Leavers 2027").is_err());
    }

    #[test]
    fn refuses_to_replace_permissions_if_net2_omits_part_of_the_set() {
        assert!(with_level(r#"{"accessLevels":[1]}"#, 2, true).is_err());
        assert!(
            with_level(
                r#"{"accessLevels":[null],"individualPermissions":[]}"#,
                2,
                true
            )
            .is_err()
        );
        assert_eq!(
            with_level(
                r#"{"AccessLevels":[1],"IndividualPermissions":[]}"#,
                1,
                true
            ),
            Ok(None)
        );
    }

    #[test]
    fn setting_a_level_replaces_the_old_one_and_keeps_door_permissions() {
        let set = r#"{"accessLevels":[1],"individualPermissions":[{"id":42,"timezoneId":3}]}"#;
        assert_eq!(
            with_level(set, 2, true),
            Ok(Some(serde_json::json!({
                "accessLevels": [2], "individualPermissions": [{"id": 42, "timezoneId": 3}],
            })))
        );
        assert_eq!(
            with_level(
                r#"{"accessLevels":[2],"individualPermissions":[]}"#,
                2,
                false
            ),
            Ok(Some(
                serde_json::json!({"accessLevels": [], "individualPermissions": []})
            ))
        );
        assert_eq!(with_level(set, 2, false), Ok(None), "already without it");
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

    #[rstest]
    #[case::bare(r#"true"#, Ok(true))]
    #[case::pascal(r#"{"UserHasTemporaryAccess": false}"#, Ok(false))]
    #[case::camel(r#"{"userHasTemporaryAccess": true}"#, Ok(true))]
    fn temporary_access_is_read_in_either_shape(
        #[case] body: &str,
        #[case] has: Result<bool, String>,
    ) {
        assert_eq!(has_temporary_access(body), has);
    }

    #[test]
    fn an_unreadable_temporary_access_reply_shows_what_net2_said() {
        let said = has_temporary_access(r#"{"expiry": "2027-01-01"}"#).unwrap_err();
        assert!(
            said.ends_with(r#"It said: {"expiry": "2027-01-01"}"#),
            "{said}"
        );
    }
}
