use url::Url;

/// Schemes a chat message may ask the system to open.
const OPENABLE: [&str; 4] = ["http", "https", "mailto", "tel"];

/// Parses a link from a message and refuses anything but web, mail and phone
/// links: `file:`, `javascript:` and app-specific schemes can do more than
/// the sender should be able to make happen. The result is normalized (spaces
/// become `%20`, and so on).
pub fn check_openable(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| "not a valid URL".to_string())?;
    if !OPENABLE.contains(&url.scheme()) {
        return Err(format!("refusing to open a {}: link", url.scheme()));
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_mail_and_phone_links_are_opened() {
        assert_eq!(check_openable("https://example.com/a b").unwrap().as_str(), "https://example.com/a%20b");
        assert_eq!(check_openable("mailto:someone@example.com").unwrap().as_str(), "mailto:someone@example.com");
        assert!(check_openable("http://example.com").is_ok());
        assert!(check_openable("tel:+15551234567").is_ok());
        for bad in
            ["file:///etc/passwd", "javascript:alert(1)", "x-apple.systempreferences:", "smb://host/share", "ftp://example.com", "not a url", "example.com", ""]
        {
            assert!(check_openable(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn scheme_case_does_not_get_around_the_check() {
        assert!(check_openable("JavaScript:alert(1)").is_err());
        assert_eq!(check_openable("HTTPS://Example.com").unwrap().scheme(), "https");
    }
}
