//! New Net2 users from a spreadsheet: the fields a row can fill, and the user
//! record each row becomes. `people_client` checks them against Net2 and
//! sends them.
use serde_json::{Map, Value, json};

/// The fields a column can be mapped to, in the mapping panel's order: what
/// each is called on screen, then other headings that mean the same. Net2's
/// custom fields follow these.
pub const FIELDS: [&[&str]; 13] = [
    &["First name", "Forename", "Given name"],
    &["Middle name", "Middle names"],
    &["Surname", "Last name", "Family name"],
    &["Department", "Dept"],
    &["Access level", "Access levels"],
    &["Activation date", "Activate date", "Valid from"],
    &["Expiration date", "Expiry date", "Expiry", "Valid until"],
    &["PIN"],
    &["Telephone", "Phone"],
    &["Extension"],
    &["Fax"],
    &["Photo", "Photo file", "Portrait", "Picture", "Image"],
    &["User ID", "Net2 ID"],
];
pub const FIRST: usize = 0;
pub const MIDDLE: usize = 1;
pub const SURNAME: usize = 2;
pub const DEPARTMENT: usize = 3;
const ACCESS_LEVELS: usize = 4;
const ACTIVATE: usize = 5;
const EXPIRY: usize = 6;
const PIN: usize = 7;
const PHONES: [(usize, &str); 3] = [(8, "telephone"), (9, "extension"), (10, "fax")];
pub const PHOTO: usize = 11;
pub const USER_ID: usize = 12;

/// One of Net2's custom fields, as `GET /users/customfieldnames` lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Custom {
    pub id: i32,
    pub name: String,
    /// None for the long notes field.
    pub max: Option<usize>,
}

/// Net2's own names and lengths, for a server that does not list them.
/// They are also the Net2 import CSV's headings, in the same order.
pub fn default_custom() -> Vec<Custom> {
    [
        "Address 1",
        "Address 2",
        "Town",
        "County",
        "Post code",
        "Home telephone",
        "Home Fax",
        "Mobile",
        "Email",
        "Position",
        "Start date",
        "Car registration",
        "Notes",
        "Personnel number",
    ]
    .into_iter()
    .zip(1..)
    .map(|(name, id)| Custom {
        id,
        name: name.into(),
        max: match id {
            1 | 2 => Some(100),
            13 => None,
            _ => Some(50),
        },
    })
    .collect()
}

/// Every mappable field, the custom ones last.
pub fn fields(custom: &[Custom]) -> Vec<Vec<String>> {
    FIELDS
        .iter()
        .map(|names| names.iter().map(|name| name.to_string()).collect())
        .chain(custom.iter().map(|field| vec![field.name.clone()]))
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Person {
    pub first: String,
    pub middle: String,
    pub last: String,
    pub department: String,
    /// Net2 access level names, as the sheet gives them.
    pub access_levels: Vec<String>,
    pub activate: Option<String>,
    pub expiry: Option<String>,
    pub photo: String,
    /// Set when this row already went to Net2: it is never sent again.
    pub user_id: String,
    body: Map<String, Value>,
}

impl Person {
    /// The row's person, with `map` giving the column of each of `fields`.
    pub fn read(row: &[String], map: &[Option<usize>], custom: &[Custom]) -> Result<Self, String> {
        let get = |field: usize| {
            map.get(field)
                .copied()
                .flatten()
                .and_then(|column| row.get(column))
                .map_or("", |text| text.trim())
        };
        let mut person = Self {
            first: get(FIRST).into(),
            middle: get(MIDDLE).into(),
            last: get(SURNAME).into(),
            department: get(DEPARTMENT).into(),
            // Several in one cell as `Working hours; Car park`: Net2's own
            // names can hold commas.
            access_levels: get(ACCESS_LEVELS)
                .split(';')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(String::from)
                .collect(),
            activate: read_date(get(ACTIVATE)).map_err(|e| format!("Activation date: {e}"))?,
            expiry: read_date(get(EXPIRY)).map_err(|e| format!("Expiration date: {e}"))?,
            // A path in the sheet still names the file the operator chose.
            photo: get(PHOTO).rsplit(['/', '\\']).next().unwrap_or("").into(),
            user_id: get(USER_ID).into(),
            body: Map::new(),
        };
        if person.first.is_empty() && person.last.is_empty() {
            return Err("No first name or surname.".into());
        }
        if let (Some(from), Some(until)) = (&person.activate, &person.expiry)
            && until < from
        {
            return Err("The expiration date is before the activation date.".into());
        }
        let pin = get(PIN);
        if !pin.bytes().all(|b| b.is_ascii_digit()) {
            return Err("A PIN is digits only.".into());
        }
        let mut body = json!({
            "firstName": person.first,
            "middleName": person.middle,
            "lastName": person.last,
            "isAlarmUser": false,
        });
        let fields = body.as_object_mut().unwrap();
        // Net2's own times: from the start of the first day to the end of
        // the last, as its user editor sets them.
        if let Some(date) = &person.activate {
            fields.insert("activateDate".into(), format!("{date}T00:00:00").into());
        }
        if let Some(date) = &person.expiry {
            fields.insert("expiryDate".into(), format!("{date}T23:59:00").into());
        }
        if !pin.is_empty() {
            fields.insert("pin".into(), pin.into());
        }
        for (field, key) in PHONES {
            if !get(field).is_empty() {
                fields.insert(key.into(), get(field).into());
            }
        }
        let mut values = Vec::new();
        for (index, field) in custom.iter().enumerate() {
            let value = get(FIELDS.len() + index);
            if value.is_empty() {
                continue;
            }
            if let Some(max) = field.max
                && value.chars().count() > max
            {
                return Err(format!(
                    "{} is longer than Net2's {max} characters.",
                    field.name
                ));
            }
            values.push(json!({ "id": field.id, "value": value }));
        }
        if !values.is_empty() {
            fields.insert("customFields".into(), values.into());
        }
        person.body = fields.clone();
        Ok(person)
    }

    pub fn name(&self) -> String {
        [&self.first, &self.middle, &self.last]
            .into_iter()
            .filter(|part| !part.is_empty())
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// What `POST /users` gets.
    pub fn body(&self) -> Value {
        Value::Object(self.body.clone())
    }
}

/// Who counts as the same person: first name and surname, ignoring case and
/// spacing. Middle names are left out, as one side often lacks them.
pub fn name_key(first: &str, last: &str) -> String {
    format!("{first} {last}")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// A date as `YYYY-MM-DD`. Taken as 2027-07-31, 2027-Jul-31 (Net2's import
/// CSV), 31/07/2027 or 31-Jul-2027, day before month, as UK sites write it.
/// Anything else is refused rather than guessed at.
pub fn read_date(text: &str) -> Result<Option<String>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    // A workbook date with a time of day, from `sheet`.
    let date = text.split_once(' ').map_or(text, |(date, _)| date);
    let parts: Vec<&str> = date.split(['-', '/', '.']).collect();
    let number = |part: &str| part.parse::<u32>().ok().filter(|_| part.len() <= 4);
    let month = |part: &str| {
        number(part).or_else(|| {
            let lower = part.to_ascii_lowercase();
            MONTHS
                .iter()
                .position(|name| lower.get(..3) == Some(name))
                .map(|index| index as u32 + 1)
        })
    };
    let (year, month, day) = match parts.as_slice() {
        [y, m, d] if y.len() == 4 => (number(y), month(m), number(d)),
        [d, m, y] if y.len() == 4 => (number(y), month(m), number(d)),
        _ => (None, None, None),
    };
    let refused = || format!("{text} is not a date like 2027-07-31 or 31/07/2027.");
    let (Some(year), Some(month), Some(day)) = (year, month, day) else {
        return Err(refused());
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1..=12 => 31,
        _ => 0,
    };
    if !(1900..=9999).contains(&year) || day == 0 || day > days {
        return Err(refused());
    }
    Ok(Some(format!("{year:04}-{month:02}-{day:02}")))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn row(cells: &[&str]) -> Vec<String> {
        cells.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn dates_are_read_day_first_and_never_guessed() {
        for (text, date) in [
            ("2027-07-31", "2027-07-31"),
            ("2027-Jul-31", "2027-07-31"),
            ("1999-Jan-01", "1999-01-01"),
            ("31/07/2027", "2027-07-31"),
            ("1/2/2027", "2027-02-01"),
            ("31-July-2027", "2027-07-31"),
            ("29/02/2028", "2028-02-29"),
            ("2027-07-31 00:00:00", "2027-07-31"),
        ] {
            assert_eq!(read_date(text), Ok(Some(date.into())), "{text}");
        }
        assert_eq!(read_date("  "), Ok(None));
        for bad in [
            "07/31/2027",
            "29/02/2027",
            "31/04/2027",
            "2027-13-01",
            "27-07-31",
            "0001-01-01",
            "31 July 2027",
            "soon",
            "2027-Ju-31",
        ] {
            assert!(read_date(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_row_becomes_the_user_net2_is_sent() {
        let custom = default_custom();
        let headers = row(&[
            "Surname",
            "First name",
            "Expiry",
            "PIN",
            "Personnel number",
            "Photo",
            "Access levels",
            "Town",
            "Fax",
        ]);
        let map: Vec<_> = fields(&custom)
            .iter()
            .map(|names| crate::sheet::guess(&headers, names))
            .collect();
        let person = Person::read(
            &row(&[
                "Patel",
                "Riya",
                "31/07/2027",
                "0123",
                "S1234",
                r"C:\photos\riya.jpg",
                " Working hours, weekdays ;; Car park ",
                "",
                "0",
            ]),
            &map,
            &custom,
        )
        .unwrap();
        assert_eq!(
            (person.name().as_str(), person.photo.as_str()),
            ("Riya Patel", "riya.jpg")
        );
        assert_eq!(
            person.access_levels,
            ["Working hours, weekdays", "Car park"]
        );
        assert_eq!(
            person.body(),
            json!({
                "firstName": "Riya", "middleName": "", "lastName": "Patel", "isAlarmUser": false,
                "expiryDate": "2027-07-31T23:59:00", "pin": "0123", "fax": "0",
                "customFields": [{"id": 14, "value": "S1234"}],
            })
        );
        let read = |cells: &[&str]| Person::read(&row(cells), &map, &custom);
        assert!(
            read(&["", "", "", "", "", "", "", "", ""])
                .unwrap_err()
                .contains("No first name")
        );
        assert!(
            read(&["Doe", "", "07/31/2027", "", "", "", "", "", ""])
                .unwrap_err()
                .starts_with("Expiration date:")
        );
        assert!(read(&["Doe", "", "", "12a4", "", "", "", "", ""]).is_err());
        let town = "x".repeat(51);
        assert_eq!(
            read(&["Doe", "", "", "", "", "", "", &town, ""]).unwrap_err(),
            "Town is longer than Net2's 50 characters."
        );
    }

    #[test]
    fn an_activation_after_the_expiry_is_refused() {
        let map = [Some(0), None, None, None, None, Some(1), Some(2)];
        let read = |from: &str, until: &str| Person::read(&row(&["Ada", from, until]), &map, &[]);
        assert!(read("2027-01-02", "2027-01-01").is_err());
        let one_day = read("2027-01-01", "2027-01-01").unwrap().body();
        assert_eq!(one_day["activateDate"], "2027-01-01T00:00:00");
        assert_eq!(read("", "").unwrap().body().get("expiryDate"), None);
    }

    #[test]
    fn the_same_person_is_found_whatever_the_spacing_or_case() {
        assert_eq!(name_key(" Riya ", "PATEL"), name_key("riya", "Patel"));
        assert_eq!(name_key("Mary Ann", "Lee"), name_key("Mary  Ann", "lee"));
        assert_ne!(name_key("Ann", "Lee"), name_key("Anne", "Lee"));
    }

    use proptest::prelude::*;

    proptest! {
        /// Whatever a date column holds, a date that is accepted is real and
        /// reads back as itself.
        #[test]
        fn an_accepted_date_is_real(text in r"[0-9A-Za-z/.\- ]{0,14}") {
            if let Ok(Some(date)) = read_date(&text) {
                prop_assert_eq!(read_date(&date), Ok(Some(date.clone())));
                prop_assert_eq!(date.len(), 10);
            }
        }
    }
}
