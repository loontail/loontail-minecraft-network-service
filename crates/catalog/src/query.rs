//! Parsing of the catalog query string. The native contract keeps only the
//! optional `locale=` param (relations are always inlined). We parse the raw
//! query because the handlers receive it via `OriginalUri`.

/// The requested locale, if any. Everything else in the query is ignored.
#[derive(Debug, Default, Clone)]
pub struct CatalogQuery {
    pub locale: Option<String>,
}

impl CatalogQuery {
    /// Parse a raw query string (without the leading `?`). Recognizes `locale=<x>`.
    pub fn parse(raw: &str) -> Self {
        let mut locale = None;
        for pair in raw.split('&').filter(|p| !p.is_empty()) {
            let (key, value) = match pair.split_once('=') {
                Some((k, v)) => (k, Some(v)),
                None => (pair, None),
            };
            if percent_decode(key) == "locale" {
                locale = value.map(percent_decode).filter(|v| !v.is_empty());
            }
        }
        CatalogQuery { locale }
    }
}

/// Minimal percent-decoding for query keys/values: turns `%XX` escapes and `+`
/// back into their literals. Good enough for the small, well-formed query strings
/// the launcher emits.
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
    fn parses_locale() {
        let q = CatalogQuery::parse("locale=en");
        assert_eq!(q.locale.as_deref(), Some("en"));
    }

    #[test]
    fn empty_query_has_no_locale() {
        let q = CatalogQuery::parse("");
        assert!(q.locale.is_none());
    }

    #[test]
    fn empty_locale_value_is_none() {
        let q = CatalogQuery::parse("locale=");
        assert!(q.locale.is_none());
    }
}
