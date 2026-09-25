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
/// The longest side of a photo Tusk converts. Plenty for an ID card.
pub const MAX_EDGE: u32 = 1200;

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

const NOT_SIGNED_IN: &str = "Not signed in to Net2. Connect again.";
const WRONG_OPERATOR: &str = "Net2 didn't accept the operator name or password.";

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
        401 => NOT_SIGNED_IN.to_string(),
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

/// A 401 while signing in means the operator, not an expired session.
pub fn sign_in_failure(mut problem: Failure) -> Failure {
    if problem.expired {
        problem.message = problem.message.replacen(NOT_SIGNED_IN, WRONG_OPERATOR, 1);
    }
    problem
}

/// The sign-in reply's access token. It goes into an Authorization header,
/// where anything but visible ASCII makes the browser throw.
pub fn access_token(reply: &str) -> Option<String> {
    serde_json::from_str::<Value>(reply)
        .ok()?
        .get("access_token")?
        .as_str()
        .filter(|token| !token.is_empty() && token.bytes().all(|b| b.is_ascii_graphic()))
        .map(str::to_string)
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
/// silently be the same person as `123.jpg`. Any extension is accepted:
/// whether the browser can decode the contents is found out when reading it.
pub fn portrait_user_id(filename: &str) -> Result<i32, String> {
    let (stem, _) = filename.rsplit_once('.').ok_or(NAME_RULE)?;
    if stem.is_empty() || stem.starts_with('0') || !stem.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(NAME_RULE.into());
    }
    stem.parse::<i32>().map_err(|_| NAME_RULE.into())
}

/// Whether the file can go to Net2 exactly as it is; anything else is
/// converted. Only a JPG: Net2 stores a PNG but shows no picture for it.
/// `header` need only be the first few kilobytes; `size` is the whole file.
pub fn check_portrait(filename: &str, header: &[u8], size: usize) -> Result<(), String> {
    if size == 0 || size > MAX_IMAGE_BYTES {
        return Err("Images must be no larger than 3.5 MB.".into());
    }
    let lower = filename.to_ascii_lowercase();
    let jpeg = header.starts_with(&[0xFF, 0xD8, 0xFF]);
    if !(jpeg && (lower.ends_with(".jpg") || lower.ends_with(".jpeg"))) {
        return Err("The file is not the JPG its name says.".into());
    }
    match imagesize::blob_size(header) {
        Ok(size) if size.width as u64 * size.height as u64 > MAX_PIXELS => {
            Err("Images must be no larger than 40 megapixels.".into())
        }
        Ok(_) => Ok(()),
        Err(_) => Err("The image size could not be read.".into()),
    }
}

/// The size to redraw a photo at: its own, or scaled down so the longest side
/// is `MAX_EDGE`. Never enlarged.
pub fn fit(width: u32, height: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= MAX_EDGE {
        return (width, height);
    }
    let scale = |side: u32| {
        ((side as u64 * MAX_EDGE as u64 + longest as u64 / 2) / longest as u64).max(1) as u32
    };
    (scale(width), scale(height))
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

    /// A JPEG start and frame header, which is all `imagesize` reads.
    fn jpeg(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xC0, 0, 11, 8];
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&[1, 1, 0x11, 0]);
        bytes
    }

    fn tiny_jpeg() -> Vec<u8> {
        jpeg(1, 1)
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
        let operator = sign_in_failure(status_failure(401, None, false, r#"{"error":"nope"}"#));
        assert!(operator.message.starts_with(WRONG_OPERATOR));
        assert!(operator.message.contains("nope"));
        assert_eq!(sign_in_failure(rejected.clone()), rejected);
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
        assert_eq!(portrait_user_id("8.webp"), Ok(8));
        for bad in [
            "00123.jpg",
            "0.jpg",
            "2147483648.jpg",
            "person.jpg",
            "12345",
            "12 345.jpg",
            "-1.jpg",
        ] {
            assert!(portrait_user_id(bad).is_err(), "{bad}");
        }
        let jpeg = tiny_jpeg();
        assert_eq!(check_portrait("1.jpg", &jpeg, jpeg.len()), Ok(()));
        assert_eq!(check_portrait("1.JPEG", &jpeg, jpeg.len()), Ok(()));
        // Net2 takes a PNG but shows nothing, so a PNG is always converted.
        let png = tiny_png();
        assert!(check_portrait("1.png", &png, png.len()).is_err());
        assert!(check_portrait("1.jpg", &png, png.len()).is_err());
        assert!(check_portrait("1.png", &jpeg, jpeg.len()).is_err());
        assert!(check_portrait("1.jpg", &jpeg, 0).is_err());
        assert!(check_portrait("1.jpg", &jpeg, MAX_IMAGE_BYTES + 1).is_err());
        assert_eq!(fit(1024, 1024), (1024, 1024));
        assert_eq!(fit(4000, 3000), (1200, 900));
        assert_eq!(fit(3000, 4001), (900, 1200));
        assert_eq!(fit(20000, 5), (1200, 1));
        assert_eq!(
            duplicate_flags(&[Some(1), Some(2), Some(1), None, None]),
            [true, false, true, false, false]
        );
    }

    #[test]
    fn the_size_limits_are_net2s_to_the_byte_and_pixel() {
        let small = tiny_jpeg();
        assert_eq!(check_portrait("1.jpg", &small, 3_670_016), Ok(()));
        assert!(check_portrait("1.jpg", &small, 3_670_017).is_err());
        // 40 megapixels exactly is allowed; one row more is not
        let forty = jpeg(8000, 5000);
        assert_eq!(check_portrait("1.jpg", &forty, forty.len()), Ok(()));
        let over = jpeg(8000, 5001);
        assert!(check_portrait("1.jpg", &over, over.len()).is_err());
        // a JPEG start with no readable frame header
        assert!(check_portrait("1.jpg", &[0xFF, 0xD8, 0xFF], 3).is_err());
    }

    #[test]
    fn each_refusal_says_what_net2_meant() {
        for (status, words) in [
            (400, "refused"),
            (403, "permission"),
            (404, "Not found"),
            (413, "larger"),
            (429, "Wait a minute"),
            (500, "HTTP 500"),
        ] {
            let problem = status_failure(status, None, false, "");
            assert!(
                problem.message.contains(words),
                "{status}: {}",
                problem.message
            );
            assert_eq!(problem.halt, matches!(status, 403 | 429), "{status}");
            assert!(!problem.expired, "{status}");
        }
        // A 4xx on a write was answered, so it did not half-happen.
        assert!(!status_failure(404, None, true, "").uncertain);
        let long = format!(r#"{{"message":"{}"}}"#, "x".repeat(2000));
        assert_eq!(
            status_failure(400, None, false, &long)
                .message
                .chars()
                .count(),
            600
        );
    }

    #[test]
    fn only_a_usable_access_token_signs_in() {
        assert_eq!(
            access_token(r#"{"access_token":"abc.DEF-123_=","expires_in":3600}"#).as_deref(),
            Some("abc.DEF-123_=")
        );
        for bad in [
            r#"{"access_token":""}"#,
            r#"{"access_token":"two words"}"#,
            r#"{"access_token":"line\nbreak"}"#,
            r#"{"access_token":"café"}"#,
            r#"{"access_token":42}"#,
            r#"{"token":"abc"}"#,
            "<html>sign in</html>",
        ] {
            assert_eq!(access_token(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_middle_name_is_part_of_the_name_and_no_name_is_said_so() {
        let user = |first: &str, middle: &str, last: &str| User {
            id: 1,
            first: first.into(),
            middle: middle.into(),
            last: last.into(),
            has_image: false,
        };
        assert_eq!(user("Ada", "King", "Lovelace").name(), "Ada King Lovelace");
        assert_eq!(user("", "", "Lovelace").name(), "Lovelace");
        assert_eq!(user("", "", "").name(), "(No name)");
        assert_eq!(read_departments("not json"), vec![]);
        assert!(read_cards("{}").is_err());
    }

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn a_fitted_photo_is_never_enlarged_or_distorted(width in 1u32..100_000, height in 1u32..100_000) {
            let (w, h) = fit(width, height);
            prop_assert!(w >= 1 && h >= 1 && w <= width && h <= height);
            let longest = width.max(height);
            prop_assert_eq!(w.max(h), longest.min(MAX_EDGE));
            // each side is its share of MAX_EDGE, rounded, or 1 at the least
            let scale = f64::from(w.max(h)) / f64::from(longest);
            prop_assert!((f64::from(w) - f64::from(width) * scale).abs() <= 1.0);
            prop_assert!((f64::from(h) - f64::from(height) * scale).abs() <= 1.0);
        }

        #[test]
        fn any_positive_id_names_its_own_file(id in 1..=i32::MAX, ext in "(jpg|JPG|jpeg|png|webp)") {
            prop_assert_eq!(portrait_user_id(&format!("{id}.{ext}")), Ok(id));
            let padded = format!("0{id}.{ext}");
            prop_assert!(portrait_user_id(&padded).is_err());
        }

        /// Whatever is typed, the result is a bare https origin, and cleaning
        /// it again changes nothing.
        #[test]
        fn an_accepted_origin_is_already_clean(typed in r"\s?(https?://)?[A-Za-z0-9.:@/?#\-]{0,20}\s?") {
            if let Ok(origin) = parse_origin(&typed) {
                let host = origin.strip_prefix("https://").unwrap();
                prop_assert!(!host.is_empty() && !host.contains(['/', '@', '?', '#']));
                prop_assert_eq!(host, host.to_ascii_lowercase());
                prop_assert_eq!(parse_origin(&origin), Ok(origin.clone()));
            }
        }

        #[test]
        fn a_refusal_never_repeats_the_access_token(
            status in 400u16..600,
            token in "[A-Z0-9]{12,40}",
            detail in "[a-z ]{0,20}",
        ) {
            let body = format!(r#"{{"access_token":"{token}","refresh_token":"{token}","error":"{detail}"}}"#);
            let problem = status_failure(status, None, true, &body);
            prop_assert!(!problem.message.contains(&token));
        }

        /// Replies come from a server Tusk does not control.
        #[test]
        fn no_reply_or_file_header_makes_it_panic(text in ".{0,200}", bytes in prop::collection::vec(any::<u8>(), 0..200)) {
            let _ = (read_user(&text, 1), read_users(&text), read_cards(&text), read_departments(&text));
            let _ = (access_token(&text), status_failure(500, Some(&text), true, &text));
            let _ = check_portrait("1.jpg", &bytes, bytes.len());
            let mut jpeg = vec![0xFF, 0xD8, 0xFF];
            jpeg.extend(&bytes);
            let _ = check_portrait("1.jpg", &jpeg, jpeg.len());
        }
    }
}
