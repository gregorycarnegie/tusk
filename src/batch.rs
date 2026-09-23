//! CSV data and the assignment queue. Only Card Number changes on export.
//! A queue loaded from Net2 is direct: each tap is saved to Net2 first, and
//! only counts as assigned once Net2 has accepted it.
use crate::{net2::User, token::Token};

#[derive(Clone, Copy, Default, PartialEq)]
pub enum Format {
    #[default]
    Decimal,
    Hex,
}

impl Format {
    pub fn value(self, token: &Token) -> String {
        match self {
            Self::Decimal => token.number.to_string(),
            Self::Hex => token.hex.clone(),
        }
    }
}

pub struct Row {
    pub fields: Vec<String>,
    pub assigned: Option<Token>,
    pub skipped: bool,
    /// Net2 may or may not have saved this card; left out of the queue.
    pub unsure: bool,
    eligible: bool,
}

pub struct Batch {
    pub headers: Vec<String>,
    pub rows: Vec<Row>,
    pub format: Format,
    pub running: bool,
    pub notice: String,
    pub dirty: bool,
    /// Net2 user IDs, one per row, for a queue loaded from Net2.
    pub net2_ids: Option<Vec<i32>>,
    /// The tap waiting for Net2 to accept it.
    pub sending: Option<(usize, Token)>,
    /// Give a card to people who already have one.
    pub replace: bool,
    card_column: usize,
    first_column: usize,
    surname_column: usize,
    history: Vec<usize>,
    last_seen: Option<String>,
    bom: bool,
}

impl Batch {
    pub fn parse(bytes: &[u8], replace: bool, format: Format) -> Result<Self, String> {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| "Save the spreadsheet as CSV UTF-8, then choose it again.".to_string())?;
        let mut reader = csv::ReaderBuilder::new()
            .flexible(true)
            .from_reader(text.trim_start_matches('\u{feff}').as_bytes());
        let headers: Vec<String> = reader
            .headers()
            .map_err(|e| e.to_string())?
            .iter()
            .map(str::to_owned)
            .collect();
        let column = |name: &str| {
            let found: Vec<_> = headers
                .iter()
                .enumerate()
                .filter(|(_, h)| h.trim().eq_ignore_ascii_case(name))
                .map(|(i, _)| i)
                .collect();
            match found.as_slice() {
                [index] => Ok(*index),
                _ => Err(format!("The CSV must contain exactly one {name} column.")),
            }
        };
        let (first_column, surname_column, card_column) = (
            column("First name")?,
            column("Surname")?,
            column("Card Number")?,
        );
        let mut rows = Vec::new();
        for record in reader.records() {
            if rows.len() == 10_000 {
                return Err("Please split this CSV into batches of at most 10,000 people.".into());
            }
            let record = record.map_err(|e| format!("Could not read CSV: {e}"))?;
            // Net2's sample has an extra trailing comma. Keep those empty fields,
            // but reject non-empty extras or short rows that could shift a name/card.
            if record.len() < headers.len()
                || record.iter().skip(headers.len()).any(|s| !s.is_empty())
            {
                return Err(format!(
                    "CSV row {} does not match the header columns.",
                    rows.len() + 2
                ));
            }
            if record[first_column].trim().is_empty() && record[surname_column].trim().is_empty() {
                return Err(format!(
                    "CSV row {} has no first name or surname.",
                    rows.len() + 2
                ));
            }
            rows.push(Row {
                eligible: replace || record[card_column].trim().is_empty(),
                fields: record.iter().map(str::to_owned).collect(),
                assigned: None,
                skipped: false,
                unsure: false,
            });
        }
        if rows.is_empty() {
            return Err("The CSV has headers but no people to assign cards to.".into());
        }
        Ok(Self {
            headers,
            rows,
            format,
            running: false,
            notice: String::new(),
            dirty: false,
            net2_ids: None,
            sending: None,
            replace,
            card_column,
            first_column,
            surname_column,
            history: Vec::new(),
            last_seen: None,
            bom: bytes.starts_with(b"\xef\xbb\xbf"),
        })
    }

    /// A direct queue for Net2 users, in Net2 decimal. Its CSV download keeps
    /// the user IDs, so it is also a record of what was saved.
    pub fn from_users(users: &[User], replace: bool) -> Result<Self, String> {
        if users.is_empty() {
            return Err("Net2 has no users there.".into());
        }
        let mut output = csv::Writer::from_writer(Vec::new());
        let mut write = |fields: &[&str]| output.write_record(fields).map_err(|e| e.to_string());
        write(&["User ID", "First name", "Surname", "Card Number"])?;
        for user in users {
            let first = match [user.first.as_str(), &user.middle].join(" ").trim() {
                "" if user.last.is_empty() => "(No name)".to_string(),
                first => first.to_string(),
            };
            write(&[&user.id.to_string(), &first, &user.last, ""])?;
        }
        let bytes = output.into_inner().map_err(|e| e.to_string())?;
        let mut batch = Self::parse(&bytes, replace, Format::Decimal)?;
        batch.net2_ids = Some(users.iter().map(|user| user.id).collect());
        Ok(batch)
    }

    pub fn name(&self, index: usize) -> String {
        [
            self.rows[index].fields[self.first_column].trim(),
            self.rows[index].fields[self.surname_column].trim(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
    }

    pub fn current(&self) -> Option<usize> {
        self.rows
            .iter()
            .position(|r| r.eligible && r.assigned.is_none() && !r.skipped && !r.unsure)
    }

    pub fn value(&self, index: usize) -> String {
        let row = &self.rows[index];
        row.assigned
            .as_ref()
            .map(|t| self.format.value(t))
            .unwrap_or_else(|| row.fields[self.card_column].clone())
    }

    pub fn row_status(&self, index: usize) -> &'static str {
        let row = &self.rows[index];
        if row.assigned.is_some() && self.net2_ids.is_some() {
            "Saved to Net2"
        } else if row.assigned.is_some() {
            "Assigned"
        } else if row.unsure {
            "Check in Net2"
        } else if row.skipped {
            "Skipped"
        } else if !row.eligible {
            "Kept"
        } else {
            "Waiting"
        }
    }

    pub fn assigned_count(&self) -> usize {
        self.rows.iter().filter(|r| r.assigned.is_some()).count()
    }

    /// A card saved to Net2 can only be removed in Net2.
    pub fn can_undo(&self) -> bool {
        self.sending.is_none()
            && self.history.last().is_some_and(|&index| {
                self.net2_ids.is_none() || self.rows[index].assigned.is_none()
            })
    }

    pub fn resume(&mut self, card: Option<&Token>) {
        self.running = self.current().is_some();
        self.last_seen = card.map(|t| t.hex.clone());
        self.notice = if card.is_some() {
            "Remove the card already on the reader before starting.".into()
        } else {
            String::new()
        };
    }

    /// Observe changes, never repeated polling reports of the same held card.
    pub fn observe(&mut self, card: Option<&Token>) {
        let identity = card.map(|t| t.hex.clone());
        if identity == self.last_seen {
            return;
        }
        self.last_seen = identity;
        if !self.running || self.sending.is_some() {
            return;
        }
        let (Some(token), Some(index)) = (card, self.current()) else {
            return;
        };
        let value = self.format.value(token);
        // Hundreds of students: a linear scan also checks existing CSV numbers.
        // ponytail: O(n) per tap; index values if batches grow beyond a few thousand.
        let duplicate = (0..self.rows.len()).find(|&other| {
            other != index
                && (self.rows[other]
                    .assigned
                    .as_ref()
                    .is_some_and(|t| t.hex == token.hex)
                    || same_number(&self.value(other), &value, self.format))
        });
        if let Some(other) = duplicate {
            self.notice = format!(
                "Card already belongs to {} (row {}). Use a different card.",
                self.name(other),
                other + 2
            );
            return;
        }
        if self.net2_ids.is_some() {
            self.notice = format!("Saving {value} to {} in Net2…", self.name(index));
            self.sending = Some((index, token.clone()));
            return;
        }
        self.assign(index, token.clone());
        self.dirty = true;
    }

    fn assign(&mut self, index: usize, token: Token) {
        let value = self.format.value(&token);
        self.rows[index].assigned = Some(token);
        self.history.push(index);
        self.notice = format!(
            "{} {value} to {}. Remove this card before the next tap.",
            if self.net2_ids.is_some() {
                "Saved"
            } else {
                "Assigned"
            },
            self.name(index)
        );
        if self.current().is_none() {
            self.running = false;
        }
    }

    /// Net2 accepted the staged card.
    pub fn saved(&mut self) {
        if let Some((index, token)) = self.sending.take() {
            self.assign(index, token);
        }
    }

    /// Net2 did not take the staged card. The person stays next unless they
    /// already had a card (`kept`) or the outcome is unknown (`unsure`).
    pub fn refused(&mut self, message: String, kept: bool, unsure: bool, stop: bool) {
        let Some((index, _)) = self.sending.take() else {
            return;
        };
        self.rows[index].eligible &= !kept;
        self.rows[index].unsure = unsure;
        self.running &= !stop && self.current().is_some();
        self.notice = message;
    }

    /// The Net2 user the staged card is for.
    pub fn sending_to(&self) -> Option<(i32, String, String)> {
        let (index, token) = self.sending.as_ref()?;
        let id = *self.net2_ids.as_ref()?.get(*index)?;
        Some((id, self.name(*index), self.format.value(token)))
    }

    pub fn skip(&mut self) {
        if self.sending.is_some() {
            return;
        }
        if let Some(index) = self.current() {
            self.rows[index].skipped = true;
            self.history.push(index);
            // Nothing is lost by leaving a direct queue: Net2 already has it.
            self.dirty |= self.net2_ids.is_none();
            self.notice = format!(
                "Skipped {}. Their original Card Number is unchanged.",
                self.name(index)
            );
            if self.current().is_none() {
                self.running = false;
            }
        }
    }

    pub fn undo(&mut self) {
        if !self.can_undo() {
            return;
        }
        if let Some(index) = self.history.pop() {
            self.rows[index].assigned = None;
            self.rows[index].skipped = false;
            self.running = false;
            self.dirty |= self.net2_ids.is_none();
            self.notice = format!(
                "Returned to {}. Press Start / resume when ready.",
                self.name(index)
            );
        }
    }

    pub fn export(&self) -> Result<Vec<u8>, String> {
        let mut writer = csv::WriterBuilder::new()
            .flexible(true)
            .terminator(csv::Terminator::CRLF)
            .from_writer(Vec::new());
        writer
            .write_record(&self.headers)
            .map_err(|e| e.to_string())?;
        for (index, row) in self.rows.iter().enumerate() {
            let mut fields = row.fields.clone();
            fields[self.card_column] = self.value(index);
            writer.write_record(fields).map_err(|e| e.to_string())?;
        }
        let bytes = writer.into_inner().map_err(|e| e.to_string())?;
        Ok(if self.bom {
            [b"\xef\xbb\xbf".as_slice(), &bytes].concat()
        } else {
            bytes
        })
    }
}

fn same_number(left: &str, right: &str, format: Format) -> bool {
    if left.trim().is_empty() {
        return false;
    }
    match format {
        Format::Decimal => left.trim().parse::<u32>().ok() == right.parse::<u32>().ok(),
        Format::Hex => left
            .trim()
            .trim_start_matches('0')
            .eq_ignore_ascii_case(right.trim_start_matches('0')),
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::token::Read;

    #[test]
    fn guided_assignment_preserves_csv_and_requires_new_cards() {
        let input = "\u{feff}Surname,First name,Card Number,Notes\r\nDoe,John,,\"Comma, quote \"\" and\nnewline\"\r\nDawkins,Jane,,,\r\nKept,Student,0034935098,unchanged\r\n";
        let mut batch = Batch::parse(input.as_bytes(), false, Format::Decimal).unwrap();
        let card = Token {
            read: Read::Mifare,
            hex: "5B7D4039".into(),
            number: 34935097,
        };
        batch.resume(Some(&card));
        batch.observe(Some(&card)); // A card present before Start is not assigned.
        assert_eq!(batch.assigned_count(), 0);
        batch.observe(None);
        batch.observe(Some(&card));
        for _ in 0..20 {
            batch.observe(Some(&card));
        }
        assert_eq!(batch.assigned_count(), 1);
        assert_eq!(batch.name(batch.current().unwrap()), "Jane Dawkins");
        batch.observe(None);
        batch.observe(Some(&card)); // A duplicate after lifting it is still refused.
        assert_eq!(batch.assigned_count(), 1);
        assert!(batch.notice.contains("already belongs to John Doe"));
        let mut next = card.clone();
        next.hex = "5B7D403A".into();
        next.number += 1;
        batch.observe(Some(&next)); // Decimal collision with a preserved CSV number.
        assert!(batch.notice.contains("already belongs to Student Kept"));
        next.hex = "5B7D403B".into();
        next.number += 1;
        batch.observe(Some(&next));
        assert!(batch.current().is_none());
        let exported = batch.export().unwrap();
        let reread = Batch::parse(&exported, false, Format::Decimal).unwrap();
        assert!(exported.starts_with(b"\xef\xbb\xbf"));
        assert_eq!(reread.rows[0].fields[3], "Comma, quote \" and\nnewline");
        assert_eq!(reread.rows[1].fields.len(), 5); // Sample's trailing empty field.
        assert_eq!(reread.value(0), "34935097");
        assert_eq!(reread.value(2), "0034935098");
        batch.undo();
        assert!(!batch.running);
        assert_eq!(batch.value(1), "");
        batch.skip();
        assert_eq!(batch.row_status(1), "Skipped");
        batch.undo();
        batch.format = Format::Hex;
        assert_eq!(batch.value(0), "5B7D4039");
        assert!(
            Batch::parse(
                b"Surname,First name,Card Number\nSmith,John,12345678,\n",
                true,
                Format::Decimal
            )
            .unwrap()
            .current()
            .is_some()
        );
        for invalid in [
            "First name,Card Number\nJohn,1",
            "Surname,First name,Card Number\nDoe,John",
            "Surname,First name,Card Number\nDoe,John,1,extra",
            "Surname,First name,Card Number\n,,",
            "Surname,First name,Card Number",
        ] {
            assert!(Batch::parse(invalid.as_bytes(), false, Format::Decimal).is_err());
        }
        assert!(Batch::parse(&[0xff], false, Format::Decimal).is_err());
    }

    #[test]
    fn direct_queue_counts_a_card_only_once_net2_accepts_it() {
        let user = |id, first: &str, last: &str| User {
            id,
            first: first.into(),
            middle: String::new(),
            last: last.into(),
            has_image: false,
        };
        let users = [
            user(8, "Jane", "Doe"),
            user(9, "", ""),
            user(12, "John", "Roe"),
        ];
        let mut batch = Batch::from_users(&users, false).unwrap();
        assert_eq!(batch.name(1), "(No name)");
        let mut card = Token {
            read: Read::Mifare,
            hex: "5B7D4039".into(),
            number: 34935097,
        };
        batch.resume(None);
        batch.observe(Some(&card));
        assert_eq!(
            batch.sending_to(),
            Some((8, "Jane Doe".into(), "34935097".into()))
        );
        assert_eq!(batch.assigned_count(), 0);
        batch.skip(); // Nothing moves while Net2 is answering.
        assert_eq!(batch.current(), Some(0));
        batch.refused("Jane already has a card.".into(), true, false, false);
        assert_eq!((batch.row_status(0), batch.current()), ("Kept", Some(1)));
        batch.observe(None);
        batch.observe(Some(&card));
        batch.saved();
        assert_eq!(batch.row_status(1), "Saved to Net2");
        assert!(!batch.can_undo() && !batch.dirty);
        card.hex = "5B7D403A".into();
        card.number += 1;
        batch.observe(Some(&card));
        batch.refused("No answer.".into(), false, true, true);
        assert_eq!(batch.row_status(2), "Check in Net2");
        assert!(!batch.running && batch.current().is_none());
        let exported = String::from_utf8(batch.export().unwrap()).unwrap();
        assert!(exported.starts_with(
            "User ID,First name,Surname,Card Number\r\n8,Jane,Doe,\r\n9,(No name),,34935097\r\n"
        ));
    }
}
