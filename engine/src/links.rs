//! Plain URLs in printed text, for the ones a program did not mark as links.

const SCHEMES: [&str; 2] = ["https://", "http://"];

/// Only web addresses are ever opened, so a link to a file or a custom scheme does nothing.
pub(crate) fn is_web(url: &str) -> bool {
    SCHEMES
        .iter()
        .any(|scheme| url.starts_with(scheme) && url.len() > scheme.len())
}

/// The http or https URL that covers the character at `char_index`, if any.
pub fn url_at(text: &str, char_index: usize) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut start = 0;
    while start < chars.len() {
        let Some(scheme_len) = scheme_at(&chars, start) else {
            start += 1;
            continue;
        };
        let run_end = (start..chars.len())
            .find(|&i| !is_url_char(chars[i]))
            .unwrap_or(chars.len());
        let end = trimmed_end(&chars[start..run_end]) + start;
        if end > start + scheme_len && (start..end).contains(&char_index) {
            return Some(chars[start..end].iter().collect());
        }
        if char_index < run_end {
            return None;
        }
        // A URL inside another one's query belongs to the outer one, so the scan resumes after it.
        start = run_end.max(start + 1);
    }
    None
}

fn scheme_at(chars: &[char], at: usize) -> Option<usize> {
    SCHEMES.iter().find_map(|scheme| {
        let len = scheme.chars().count();
        let matches =
            chars.len() - at >= len && chars[at..at + len].iter().copied().eq(scheme.chars());
        matches.then_some(len)
    })
}

/// Characters a URL can never contain unescaped, and that commonly wrap one in prose or markup.
fn is_url_char(c: char) -> bool {
    !c.is_whitespace() && !c.is_control() && !matches!(c, '<' | '>' | '"' | '`')
}

/// Where the URL in `run` ends once trailing punctuation is dropped. A closing bracket or quote
/// stays when the URL opened it, as in a Wikipedia address ending in `_(film)`.
fn trimmed_end(run: &[char]) -> usize {
    let mut end = run.len();
    while let Some(&last) = run[..end].last() {
        let url = &run[..end];
        let count = |c: char| url.iter().filter(|&&x| x == c).count();
        let drop = match last {
            '.' | ',' | ';' | ':' | '!' | '?' => true,
            ')' => count(')') > count('('),
            ']' => count(']') > count('['),
            '}' => count('}') > count('{'),
            '\'' => count('\'') % 2 == 1,
            _ => false,
        };
        if !drop {
            break;
        }
        end -= 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balanced_brackets_stay_in_the_url() {
        let text = "see https://en.wikipedia.org/wiki/Heat_(film).";
        assert_eq!(
            url_at(text, 6),
            Some("https://en.wikipedia.org/wiki/Heat_(film)".to_string())
        );
    }

    #[test]
    fn a_bare_scheme_is_not_a_url() {
        assert_eq!(url_at("https:// x", 2), None);
        assert!(!is_web("https://"));
        assert!(!is_web("file:///etc/passwd"));
    }

    #[test]
    fn the_index_picks_among_several_urls() {
        let text = "http://a.com and https://b.com";
        assert_eq!(url_at(text, 2), Some("http://a.com".to_string()));
        assert_eq!(url_at(text, 20), Some("https://b.com".to_string()));
        assert_eq!(url_at(text, 14), None);
        assert_eq!(url_at(text, 500), None);
    }
}
