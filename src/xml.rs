fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(pos) = rest.find('&') {
        let (before, after) = rest.split_at(pos);
        out.push_str(before);
        let entity = match after.find(';') {
            Some(end) => &after[1..end],
            None => {
                out.push('&');
                rest = &after[1..];
                continue;
            }
        };
        let decoded = match entity {
            "quot" => '"',
            "apos" => '\'',
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            _ => {
                out.push('&');
                rest = &after[1..];
                continue;
            }
        };
        out.push(decoded);
        rest = &after[entity.len() + 2..];
    }
    out.push_str(rest);
    out
}

fn field_at(text: &str, key: &str) -> Option<String> {
    let marker = format!("Name=\"{key}\"");
    let mut search = 0;
    while let Some(found) = text[search..].find(&marker) {
        let absolute = search + found;
        let before = text[..absolute].trim_end();
        if before.ends_with("<Value") {
            let after = &text[absolute + marker.len()..];
            if let Some(rest) = after.strip_prefix(" Value=\"") {
                let end = rest.find('"')?;
                return Some(unescape(&rest[..end]));
            }
        }
        search = absolute + marker.len();
    }
    None
}

pub fn field(text: &str, key: &str) -> Result<String, String> {
    field_at(text, key).ok_or_else(|| format!("missing factory field: {key}"))
}

pub fn optional_field(text: &str, key: &str) -> Option<String> {
    field_at(text, key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_and_unescapes() {
        let doc = r#"<a><Value Name="ELAN_MAC_ADDR" Value="d05bcb9c9bf0"/><Value Name="X" Value="a&amp;b&quot;c"/></a>"#;
        assert_eq!(field(doc, "ELAN_MAC_ADDR").unwrap(), "d05bcb9c9bf0");
        assert_eq!(field(doc, "X").unwrap(), "a&b\"c");
        assert!(field(doc, "NOPE").is_err());
    }
}
