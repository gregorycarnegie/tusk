//! The Net2 Local API without the browser: server URLs, error messages, the
//! JSON records Tusk reads, and the portrait file rules. `net2_client` makes
//! the requests. Net2's nginx answers CORS for any origin, so the page talks
//! to it directly and no Tusk server is needed.
use serde_json::Value;

/// Net2's token types: the API's name, then the label Net2 shows, in Net2's
/// order. Net2's "Dual credential" has no API name, so it is not offered.
pub const TOKEN_TYPES: [(&str, &str); 9] = [
    ("Unspecified", "Unspecified"),
    ("ProxCard", "Proximity card"),
    ("ProxIsoCard", "Proximity ISO card"),
    (
        "ProxIsoCardWithoutMagstripe",
        "Proximity ISO card no magstripe",
    ),
    ("Keyfob", "Keyfob"),
    ("HandsFreeToken", "Hands free token"),
    ("HandsFreeKeyCard", "Hands free keycard"),
    ("Watchprox", "Watchprox"),
    (
        "FingerprintVerificationCard",
        "Fingerprint verification card",
    ),
];

/// The type preselected until someone picks another.
pub const DEFAULT_TOKEN_TYPE: &str = "ProxIsoCardWithoutMagstripe";

pub const MAX_IMAGE_BYTES: usize = 3 * 1024 * 1024 + 512 * 1024;
pub const MAX_PIXELS: u64 = 40_000_000;

/// A refusal by Net2, or no answer at all.
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub message: String,
    /// The write may have landed; do not silently retry.
    pub uncertain: bool,
    /// Stop the run: the session is gone or the server is rate limiting.
    pub halt: bool,
    /// The access token was refused, so the session must be dropped.
    pub expired: bool,
}

pub fn failure(message: impl Into<String>, uncertain: bool, halt: bool) -> Failure {
    Failure {
        message: message.into(),
        uncertain,
        halt,
        expired: false,
    }
}

pub const UNCERTAIN_WRITE: &str =
    "Net2 did not confirm this change. Check it in Net2 before trying again.";

/// Only a bare https origin, so credentials cannot go in clear text or to a
/// path that is not the API root.
pub fn parse_origin(value: &str) -> Result<String, String> {
    let host = value
        .trim()
        .trim_end_matches('/')
        .strip_prefix("https://")
        .filter(|host| !host.is_empty() && !host.contains(['/', '?', '#', '@', ' ', '\\']));
    host.map(|host| format!("https://{}", host.to_ascii_lowercase()))
        .ok_or_else(|| {
            "Enter the Net2 server as https://name:port, without a path such as /api/v1.".into()
        })
}

pub fn status_failure(status: u16, retry_after: Option<&str>, write: bool, body: &str) -> Failure {
    let mut message = match status {
        400 => "Net2 refused the request.".to_string(),
        401 => "Not signed in to Net2. Connect again.".to_string(),
        403 => "Your Net2 operator or integration does not have permission.".to_string(),
        404 => "Not found in Net2.".to_string(),
        413 => "The image is larger than Net2 accepts.".to_string(),
        429 => match retry_after {
            Some(seconds) => format!("Net2 is rate limiting. Try again after {seconds} seconds."),
            None => "Net2 is rate limiting. Wait a minute, then try again.".to_string(),
        },
        other => format!("Net2 returned HTTP {other}."),
    };
    // Only error fields: sign-in responses can also carry tokens.
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        let details = ["error", "error_description", "message", "Message"]
            .into_iter()
            .filter_map(|key| value.get(key).and_then(Value::as_str))
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("; ");
        if !details.is_empty() {
            message.push_str(&format!(" Net2 says: {}.", details.trim_end_matches('.')));
        }
    }
    if message.contains("invalid_client") {
        message.push_str(" Use the licence's ClientID attribute, not its Id.");
    }
    Failure {
        message: message.chars().take(600).collect(),
        uncertain: write && status >= 500,
        halt: matches!(status, 401 | 403 | 429),
        expired: status == 401,
    }
}

/// Net2 documents camelCase but its examples are PascalCase; accept either.
fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get(key).or_else(|| {
        let mut chars = key.chars();
        let pascal: String = chars
            .next()
            .map(|c| c.to_ascii_uppercase())
            .into_iter()
            .chain(chars)
            .collect();
        value.get(pascal)
    })
}

fn text(value: &Value, key: &str) -> String {
    field(value, key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

#[derive(Clone, Debug, PartialEq)]
pub struct User {
    pub id: i32,
    pub first: String,
    pub middle: String,
    pub last: String,
    pub has_image: bool,
}

impl User {
    fn from_json(value: &Value) -> Option<Self> {
        Some(Self {
            id: field(value, "id")?.as_i64()?.try_into().ok()?,
            first: text(value, "firstName"),
            middle: text(value, "middleName"),
            last: text(value, "lastName"),
            has_image: field(value, "hasImage")?.as_bool()?,
        })
    }

    pub fn name(&self) -> String {
        let name = [&self.first, &self.middle, &self.last]
            .into_iter()
            .filter(|part| !part.is_empty())
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" ");
        if name.is_empty() {
            "(No name)".into()
        } else {
            name
        }
    }
}

const UNEXPECTED: &str = "Net2 sent a reply Tusk does not understand.";

/// The user must be the one asked for, so a proxy or a wrong path cannot pass
/// off another record as theirs.
pub fn read_user(body: &str, expected: i32) -> Result<User, String> {
    serde_json::from_str::<Value>(body)
        .ok()
        .as_ref()
        .and_then(User::from_json)
        .filter(|user| user.id == expected)
        .ok_or_else(|| UNEXPECTED.into())
}

/// Users in Net2's order. A record that cannot be read fails the whole list
/// rather than quietly leaving someone out of the queue.
pub fn read_users(body: &str) -> Result<Vec<User>, String> {
    let list: Vec<Value> = serde_json::from_str(body).map_err(|_| UNEXPECTED.to_string())?;
    list.iter()
        .map(|value| User::from_json(value).ok_or_else(|| UNEXPECTED.to_string()))
        .collect()
}

pub fn read_departments(body: &str) -> Vec<(i32, String)> {
    let list: Vec<Value> = serde_json::from_str(body).unwrap_or_default();
    let mut departments: Vec<_> = list
        .iter()
        .filter_map(|value| {
            let id = field(value, "id")?.as_i64()?.try_into().ok()?;
            Some((id, text(value, "name")))
        })
        .collect();
    departments.sort_by_key(|(_, name)| name.to_lowercase());
    departments
}

/// Card numbers the user holds that are not marked lost.
pub fn read_cards(body: &str) -> Result<Vec<String>, String> {
    let list: Vec<Value> = serde_json::from_str(body).map_err(|_| UNEXPECTED.to_string())?;
    Ok(list
        .iter()
        .filter(|token| {
            !field(token, "isLost")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .map(|token| text(token, "tokenValue"))
        .filter(|value| !value.is_empty())
        .collect())
}

const NAME_RULE: &str =
    "Name the file with its Net2 user ID, like 12345.jpg, with no leading zeros.";

/// The filename is the only link between a portrait and a person, so anything
/// ambiguous is rejected rather than guessed at. `00123.jpg` would otherwise
/// silently be the same person as `123.jpg`.
pub fn portrait_user_id(filename: &str) -> Result<i32, String> {
    let (stem, extension) = filename.rsplit_once('.').ok_or(NAME_RULE)?;
    if !matches!(
        extension.to_ascii_lowercase().as_str(),
        "jpg" | "jpeg" | "png"
    ) || stem.is_empty()
        || stem.starts_with('0')
        || !stem.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(NAME_RULE.into());
    }
    stem.parse::<i32>().map_err(|_| NAME_RULE.into())
}

/// `header` need only be the first few kilobytes; `size` is the whole file.
pub fn check_portrait(filename: &str, header: &[u8], size: usize) -> Result<(), String> {
    if size == 0 || size > MAX_IMAGE_BYTES {
        return Err("Images must be no larger than 3.5 MB.".into());
    }
    let lower = filename.to_ascii_lowercase();
    let jpeg = header.starts_with(&[0xFF, 0xD8, 0xFF]);
    let png = header.starts_with(b"\x89PNG\r\n\x1a\n");
    if !(png && lower.ends_with(".png")
        || jpeg && (lower.ends_with(".jpg") || lower.ends_with(".jpeg")))
    {
        return Err("The file is not the JPG or PNG its name says.".into());
    }
    match imagesize::blob_size(header) {
        Ok(size) if size.width as u64 * size.height as u64 > MAX_PIXELS => {
            Err("Images must be no larger than 40 megapixels.".into())
        }
        Ok(_) => Ok(()),
        Err(_) => Err("The image size could not be read.".into()),
    }
}

/// True for every copy of an ID that appears more than once.
pub fn duplicate_flags(ids: &[Option<i32>]) -> Vec<bool> {
    ids.iter()
        .map(|id| id.is_some() && ids.iter().filter(|other| *other == id).count() > 1)
        .collect()
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// Smallest header `imagesize` can read: a 1x1 PNG.
    fn tiny_png() -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0]);
        bytes
    }

    #[test]
    fn origins_are_bare_https() {
        assert_eq!(
            parse_origin(" https://Net2:8443/ ").as_deref(),
            Ok("https://net2:8443")
        );
        for bad in [
            "http://net2:8443",
            "https://net2:8443/api/v1",
            "https://user@net2",
            "https://",
            "net2:8443",
        ] {
            assert!(parse_origin(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn errors_keep_details_but_not_tokens() {
        let rejected = status_failure(
            400,
            None,
            false,
            r#"{"message":"invalid_client","access_token":"private"}"#,
        );
        assert!(rejected.message.contains("invalid_client"));
        assert!(rejected.message.contains("ClientID"));
        assert!(!rejected.message.contains("private"));
        assert!(!rejected.halt && !rejected.uncertain);
        let expired = status_failure(401, None, false, "");
        assert!(expired.halt && expired.expired);
        let limited = status_failure(429, Some("60"), false, "<html>proxy</html>");
        assert!(limited.message.contains("60 seconds") && !limited.message.contains("html"));
        assert!(status_failure(502, None, true, "").uncertain);
        assert!(!status_failure(502, None, false, "").uncertain);
    }

    #[test]
    fn records_read_in_either_case() {
        let user = read_user(
            r#"{"id":7,"firstName":"Ada","middleName":" ","lastName":"Lovelace","hasImage":false}"#,
            7,
        )
        .unwrap();
        assert_eq!(user.name(), "Ada Lovelace");
        assert!(
            read_user(r#"{"Id":7,"FirstName":"Ada","HasImage":true}"#, 7)
                .unwrap()
                .has_image
        );
        assert!(read_user(r#"{"id":8,"hasImage":false}"#, 7).is_err());
        assert!(read_users(r#"[{"id":1,"hasImage":false},{"name":"?"}]"#).is_err());
        assert_eq!(read_users("[]"), Ok(vec![]));
        assert_eq!(
            read_departments(r#"[{"id":2,"name":"year 8"},{"id":1,"name":"Year 7"}]"#),
            vec![(1, "Year 7".into()), (2, "year 8".into())]
        );
        assert_eq!(
            read_cards(
                r#"[{"tokenValue":"111","isLost":true},{"tokenValue":"222","isLost":false}]"#
            ),
            Ok(vec!["222".into()])
        );
    }

    #[test]
    fn portraits_need_an_unambiguous_id_and_honest_contents() {
        assert_eq!(portrait_user_id("12345.JPG"), Ok(12345));
        assert_eq!(portrait_user_id("7.jpeg"), Ok(7));
        for bad in [
            "00123.jpg",
            "0.jpg",
            "2147483648.jpg",
            "person.jpg",
            "24.svg",
            "12345",
            "12 345.jpg",
            "-1.jpg",
        ] {
            assert!(portrait_user_id(bad).is_err(), "{bad}");
        }
        let png = tiny_png();
        assert_eq!(check_portrait("1.png", &png, png.len()), Ok(()));
        assert!(check_portrait("1.jpg", &png, png.len()).is_err());
        assert!(check_portrait("1.png", &[0xFF, 0xD8, 0xFF], 3).is_err());
        assert!(check_portrait("1.png", &png, 0).is_err());
        assert!(check_portrait("1.png", &png, MAX_IMAGE_BYTES + 1).is_err());
        assert_eq!(
            duplicate_flags(&[Some(1), Some(2), Some(1), None, None]),
            [true, false, true, false, false]
        );
    }
}
