//! Spreadsheets as rows of text: CSV, tab-separated text, and the first sheet
//! of an Excel or OpenDocument workbook. Whatever came in goes out as CSV.
use calamine::{Data, Reader};

#[derive(Clone, Debug, PartialEq)]
pub struct Sheet {
    pub headers: Vec<String>,
    /// Each as long as `headers`, or longer only by empty fields.
    pub rows: Vec<Vec<String>>,
    /// Write a byte order mark, so Excel reads the CSV as UTF-8.
    pub bom: bool,
}

pub const MAX_ROWS: usize = 10_000;

pub fn read(filename: &str, bytes: &[u8]) -> Result<Sheet, String> {
    // A workbook is a zip (xlsx, xlsm, xlsb, ods) or an OLE file (xls),
    // whatever it is called.
    let mut sheet = if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"\xd0\xcf\x11\xe0") {
        workbook(bytes)?
    } else {
        let lower = filename.to_ascii_lowercase();
        let tabs = lower.ends_with(".tsv") || lower.ends_with(".tab");
        text(bytes, if tabs { b'\t' } else { b',' })?
    };
    // Spreadsheets often carry formatted but empty rows below the data.
    while sheet
        .rows
        .last()
        .is_some_and(|row| row.iter().all(|cell| cell.trim().is_empty()))
    {
        sheet.rows.pop();
    }
    if sheet.rows.len() > MAX_ROWS {
        return Err("Please split this sheet into batches of at most 10,000 people.".into());
    }
    Ok(sheet)
}

fn text(bytes: &[u8], delimiter: u8) -> Result<Sheet, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        "Save the spreadsheet as CSV UTF-8 or as an Excel workbook, then choose it again."
            .to_string()
    })?;
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(text.trim_start_matches('\u{feff}').as_bytes());
    let headers: Vec<String> = reader
        .headers()
        .map_err(|e| e.to_string())?
        .iter()
        .map(str::to_owned)
        .collect();
    let mut rows = Vec::new();
    for record in reader.records() {
        if rows.len() > MAX_ROWS {
            break;
        }
        let record = record.map_err(|e| format!("Could not read the sheet: {e}"))?;
        // Net2's sample has an extra trailing comma. Keep those empty fields,
        // but reject non-empty extras or short rows that could shift a name/card.
        if record.len() < headers.len() || record.iter().skip(headers.len()).any(|s| !s.is_empty())
        {
            return Err(format!(
                "Row {} does not match the header columns.",
                rows.len() + 2
            ));
        }
        rows.push(record.iter().map(str::to_owned).collect());
    }
    Ok(Sheet {
        headers,
        rows,
        bom: bytes.starts_with(b"\xef\xbb\xbf"),
    })
}

fn workbook(bytes: &[u8]) -> Result<Sheet, String> {
    let unreadable = |e: calamine::Error| format!("Could not read that workbook: {e}");
    let mut book =
        calamine::open_workbook_auto_from_rs(std::io::Cursor::new(bytes)).map_err(unreadable)?;
    let range = book
        .worksheet_range_at(0)
        .ok_or("That workbook has no sheets.")?
        .map_err(unreadable)?;
    let mut rows = range.rows().map(|row| row.iter().map(cell).collect());
    Ok(Sheet {
        headers: rows
            .next()
            .ok_or("The first sheet of that workbook is empty.")?,
        rows: rows.take(MAX_ROWS + 1).collect(),
        bom: true,
    })
}

/// A cell as it would read in a CSV: whole numbers without `.0`, so a card
/// or user ID survives, and dates as `YYYY-MM-DD`.
fn cell(data: &Data) -> String {
    match data {
        Data::Empty => String::new(),
        Data::String(text) | Data::DateTimeIso(text) | Data::DurationIso(text) => text.clone(),
        Data::Int(number) => number.to_string(),
        Data::Float(number) if number.fract() == 0.0 && number.abs() < 1e15 => {
            (*number as i64).to_string()
        }
        Data::Float(number) => number.to_string(),
        Data::Bool(value) => (if *value { "TRUE" } else { "FALSE" }).into(),
        Data::DateTime(date) if date.is_datetime() => {
            let (year, month, day, hour, minute, second, _) = date.to_ymd_hms_milli();
            match (hour, minute, second) {
                (0, 0, 0) => format!("{year:04}-{month:02}-{day:02}"),
                _ => format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}"),
            }
        }
        Data::DateTime(duration) => duration.as_f64().to_string(),
        Data::Error(error) => error.to_string(),
    }
}

/// Lower case letters and digits only, so `First name`, `FIRST_NAME` and
/// `Firstname` all compare equal.
fn simple(text: &str) -> String {
    text.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The one column whose heading is any of `names`. None when no heading or
/// more than one does, so a guess is never a coin toss.
pub fn guess(headers: &[String], names: &[impl AsRef<str>]) -> Option<usize> {
    let names: Vec<String> = names.iter().map(|name| simple(name.as_ref())).collect();
    let mut found = headers
        .iter()
        .enumerate()
        .filter(|(_, header)| names.contains(&simple(header)))
        .map(|(index, _)| index);
    match (found.next(), found.next()) {
        (Some(index), None) => Some(index),
        _ => None,
    }
}

/// A column's name in a picker: its heading, or its spreadsheet letter.
pub fn column_name(headers: &[String], index: usize) -> String {
    match headers.get(index).map(|h| h.trim()) {
        Some(header) if !header.is_empty() => header.to_string(),
        _ => {
            let (mut letters, mut n) = (String::new(), index + 1);
            while n > 0 {
                letters.insert(0, (b'A' + ((n - 1) % 26) as u8) as char);
                n = (n - 1) / 26;
            }
            format!("Column {letters}")
        }
    }
}

pub fn write_csv(
    headers: &[String],
    rows: impl IntoIterator<Item = Vec<String>>,
    bom: bool,
) -> Result<Vec<u8>, String> {
    let mut writer = csv::WriterBuilder::new()
        .flexible(true)
        .terminator(csv::Terminator::CRLF)
        .from_writer(Vec::new());
    writer.write_record(headers).map_err(|e| e.to_string())?;
    for row in rows {
        writer.write_record(row).map_err(|e| e.to_string())?;
    }
    let bytes = writer.into_inner().map_err(|e| e.to_string())?;
    Ok(if bom {
        [b"\xef\xbb\xbf".as_slice(), &bytes].concat()
    } else {
        bytes
    })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn strings(row: &[&str]) -> Vec<String> {
        row.iter().map(|s| s.to_string()).collect()
    }

    /// The smallest xlsx Excel opens: one sheet, inline strings, a number,
    /// a whole number stored as a float, and a date.
    fn xlsx() -> Vec<u8> {
        use std::io::Write;
        let files = [
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
            ),
            (
                "xl/workbook.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="People" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#,
            ),
            (
                "xl/styles.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cellXfs count="2"><xf numFmtId="0"/><xf numFmtId="14" applyNumberFormat="1"/></cellXfs></styleSheet>"#,
            ),
            (
                "xl/worksheets/sheet1.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
<row r="1"><c r="A1" t="inlineStr"><is><t>Surname</t></is></c><c r="B1" t="inlineStr"><is><t>Forename</t></is></c><c r="C1" t="inlineStr"><is><t>Card</t></is></c><c r="D1" t="inlineStr"><is><t>Expiry</t></is></c></row>
<row r="2"><c r="A2" t="inlineStr"><is><t>Doe</t></is></c><c r="B2" t="inlineStr"><is><t>Jane</t></is></c><c r="C2"><v>34935097</v></c><c r="D2" s="1"><v>46234</v></c></row>
<row r="3"><c r="A3" t="inlineStr"><is><t>Roe</t></is></c><c r="C3"><v>1.5</v></c></row>
<row r="4"><c r="A4" t="inlineStr"><is><t></t></is></c></row>
</sheetData></worksheet>"#,
            ),
        ];
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, body) in files {
            zip.start_file(
                name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn a_workbook_reads_like_the_csv_it_would_save_as() {
        // Named .csv on purpose: the contents decide.
        let sheet = read("people.csv", &xlsx()).unwrap();
        assert_eq!(
            sheet.headers,
            strings(&["Surname", "Forename", "Card", "Expiry"])
        );
        assert_eq!(
            sheet.rows,
            [
                strings(&["Doe", "Jane", "34935097", "2026-07-31"]),
                strings(&["Roe", "", "1.5", ""]),
            ],
            "the blank last row is dropped"
        );
        assert!(sheet.bom);
        assert!(read("x.xlsx", b"PK\x03\x04 not really").is_err());
    }

    #[test]
    fn text_sheets_keep_every_field_and_say_which_row_is_wrong() {
        let tabs = read("people.TSV", b"Surname\tFirst name\nDoe\tJane, Q\n").unwrap();
        assert_eq!(tabs.rows, [strings(&["Doe", "Jane, Q"])]);
        assert!(!tabs.bom);
        let csv = read("p.csv", "\u{feff}A,B\n1,2,\n,,\n".as_bytes()).unwrap();
        assert_eq!(csv.headers, strings(&["A", "B"]));
        assert_eq!(csv.rows, [strings(&["1", "2", ""])]);
        assert!(csv.bom);
        assert_eq!(
            read("p.csv", b"A,B\n1\n").unwrap_err(),
            "Row 2 does not match the header columns."
        );
        assert!(read("p.csv", b"A,B\n1,2,3\n").is_err());
        assert!(read("p.csv", &[0xff]).is_err());
        let rows = |n: usize| read("p.csv", format!("A\n{}", "x\n".repeat(n)).as_bytes());
        assert_eq!(rows(MAX_ROWS).unwrap().rows.len(), MAX_ROWS);
        assert!(rows(MAX_ROWS + 1).is_err());
    }

    #[test]
    fn a_guess_needs_exactly_one_heading_that_means_the_field() {
        let headers = strings(&["SURNAME", "first_name", "Card Number", "card number", ""]);
        assert_eq!(guess(&headers, &["Surname", "Last name"]), Some(0));
        assert_eq!(guess(&headers, &["First name"]), Some(1));
        assert_eq!(
            guess(&headers, &["Card Number"]),
            None,
            "two columns claim it"
        );
        assert_eq!(guess(&headers, &["Photo"]), None);
        assert_eq!(column_name(&headers, 0), "SURNAME");
        assert_eq!(column_name(&headers, 4), "Column E");
        assert_eq!(column_name(&headers, 26), "Column AA");
    }

    #[test]
    fn a_written_csv_reads_back_the_same() {
        let headers = strings(&["Name", "Notes"]);
        let rows = vec![strings(&["Éva", "comma, \"quote\"\nline"])];
        let bytes = write_csv(&headers, rows.clone(), true).unwrap();
        assert!(bytes.starts_with(b"\xef\xbb\xbf"));
        let back = read("x.csv", &bytes).unwrap();
        assert_eq!((back.headers, back.rows), (headers, rows));
    }

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn no_file_makes_reading_panic(bytes in prop::collection::vec(any::<u8>(), 0..300), zip: bool) {
            let mut bytes = bytes;
            if zip {
                bytes.splice(0..0, *b"PK\x03\x04");
            }
            let _ = read("x.xlsx", &bytes);
        }
    }
}
