/// 공통 명세에서 고정한 Unicode White_Space 25자. 기본 공백 함수로 대체하지 않는다.
pub fn is_white_space(c: char) -> bool {
    matches!(c, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{0085}' | '\u{00a0}'
        | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}'
        | '\u{202f}' | '\u{205f}' | '\u{3000}')
}

pub fn is_unanswered(value: &str) -> bool {
    value.chars().all(is_white_space)
}

pub fn is_valid_id(value: &str) -> bool {
    if value.is_empty() || value.len() > 64 {
        return false;
    }
    let mut previous_hyphen = true;
    for byte in value.bytes() {
        if byte == b'-' {
            if previous_hyphen {
                return false;
            }
            previous_hyphen = true;
        } else if byte.is_ascii_lowercase() || byte.is_ascii_digit() {
            previous_hyphen = false;
        } else {
            return false;
        }
    }
    !previous_hyphen
}

pub fn is_valid_answer(value: &str) -> bool {
    !is_unanswered(value)
        && !value.chars().next().is_some_and(is_white_space)
        && !value.chars().last().is_some_and(is_white_space)
        && !value
            .chars()
            .any(|c| matches!(c, '\n' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}'))
        && value
            .split(is_white_space)
            .filter(|word| !word.is_empty())
            .count()
            <= 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_white_space_membership_and_exclusions() {
        let expected = [
            9, 10, 11, 12, 13, 32, 133, 160, 5760, 8192, 8193, 8194, 8195, 8196, 8197, 8198, 8199,
            8200, 8201, 8202, 8232, 8233, 8239, 8287, 12288,
        ];
        let actual: Vec<u32> = (0..=0x10ffff)
            .filter_map(char::from_u32)
            .filter(|c| is_white_space(*c))
            .map(u32::from)
            .collect();
        assert_eq!(actual, expected);
        assert!(is_unanswered(""));
        for cp in expected {
            assert!(is_unanswered(&char::from_u32(cp).unwrap().to_string()));
        }
        for cp in [0x1c, 0x1d, 0x1e, 0x1f, 0x200b, 0xfeff] {
            let c = char::from_u32(cp).unwrap();
            assert!(!is_unanswered(&format!(" {c} ")));
        }
    }

    #[test]
    fn id_boundaries_use_ascii_and_complete_segments() {
        for value in ["a", "0", "a-0", &"x".repeat(64)] {
            assert!(is_valid_id(value));
        }
        for value in [
            "",
            "Upper",
            "a_b",
            "a--b",
            "-a",
            "a-",
            "a b",
            "a\n",
            "a\0",
            "é",
            "ａ",
            &"x".repeat(65),
        ] {
            assert!(!is_valid_id(value));
        }
    }

    #[test]
    fn invalid_answer_layout_is_rejected_without_normalization() {
        for value in [
            "",
            "\u{0085}",
            " \t",
            " value",
            "value ",
            "one\ntwo",
            "one two three four five",
        ] {
            assert!(!is_valid_answer(value));
        }
        assert!(is_valid_answer("one two three four"));
        assert!(is_valid_answer("\u{200b}"));
    }
}
