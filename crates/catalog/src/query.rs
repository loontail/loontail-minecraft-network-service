//! Parsing of the launcher's Strapi-style query string. The launcher sends
//! `populate[<field>]=true` for each relation it wants inlined plus an optional
//! `locale=`. We parse the raw query (axum's `Query` can't model the bracketed
//! keys) into a small struct the handlers consume.

use std::collections::HashSet;

/// Which relations to inline, plus the requested locale. An empty populate set
/// means "no relations" (matching Strapi: unpopulated relations are absent).
#[derive(Debug, Default, Clone)]
pub struct CatalogQuery {
    populate: HashSet<String>,
    pub locale: Option<String>,
}

impl CatalogQuery {
    /// Parse a raw query string (without the leading `?`). Recognizes
    /// `populate[field]=true` and `locale=<x>`. A `populate[field]` with a
    /// non-`true`/non-`false` value is treated as enabled (Strapi accepts bare
    /// `populate[field]` to mean "on").
    pub fn parse(raw: &str) -> Self {
        let mut populate = HashSet::new();
        let mut locale = None;

        for pair in raw.split('&').filter(|p| !p.is_empty()) {
            let (key, value) = match pair.split_once('=') {
                Some((k, v)) => (k, Some(v)),
                None => (pair, None),
            };
            let key = percent_decode(key);
            let value = value.map(percent_decode);

            if let Some(field) = key
                .strip_prefix("populate[")
                .and_then(|rest| rest.strip_suffix(']'))
            {
                let enabled = match value.as_deref() {
                    Some("false") | Some("0") => false,
                    _ => true, // `true`, `1`, or bare presence ⇒ populate
                };
                if enabled && !field.is_empty() {
                    populate.insert(field.to_string());
                }
            } else if key == "locale" {
                locale = value.filter(|v| !v.is_empty());
            }
        }

        CatalogQuery { populate, locale }
    }

    pub fn wants(&self, field: &str) -> bool {
        self.populate.contains(field)
    }

    pub fn wants_keywords(&self) -> bool {
        self.wants("keywords")
    }

    pub fn wants_servers(&self) -> bool {
        self.wants("servers")
    }

    pub fn wants_screenshots(&self) -> bool {
        self.wants("screenshots")
    }

    pub fn wants_background(&self) -> bool {
        self.wants("background")
    }

    pub fn wants_poster(&self) -> bool {
        self.wants("poster")
    }

    pub fn wants_title_image(&self) -> bool {
        self.wants("titleImage")
    }
}

/// Minimal percent-decoding for query keys/values: turns `%5B`/`%5D` (the
/// brackets in `populate[...]`) and `+` back into their literals. Good enough for
/// the small, well-formed query strings the launcher emits.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = hex_val(bytes[i + 1]);
                let lo = hex_val(bytes[i + 2]);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push(hi << 4 | lo);
                    i += 3;
                    continue;
                }
                out.push(bytes[i]);
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_launcher_populate_and_locale() {
        let raw = "populate[screenshots]=true&populate[background]=true&populate[poster]=true&populate[titleImage]=true&populate[keywords]=true&populate[servers]=true&locale=en";
        let q = CatalogQuery::parse(raw);
        assert!(q.wants_screenshots());
        assert!(q.wants_background());
        assert!(q.wants_poster());
        assert!(q.wants_title_image());
        assert!(q.wants_keywords());
        assert!(q.wants_servers());
        assert_eq!(q.locale.as_deref(), Some("en"));
    }

    #[test]
    fn decodes_bracketed_keys() {
        let q = CatalogQuery::parse("populate%5Bkeywords%5D=true");
        assert!(q.wants_keywords());
    }

    #[test]
    fn empty_query_populates_nothing() {
        let q = CatalogQuery::parse("");
        assert!(!q.wants_keywords());
        assert!(q.locale.is_none());
    }

    #[test]
    fn explicit_false_disables() {
        let q = CatalogQuery::parse("populate[keywords]=false");
        assert!(!q.wants_keywords());
    }
}
